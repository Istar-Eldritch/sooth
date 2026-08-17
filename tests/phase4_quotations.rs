//! Phase 4 Slice 7a/7b goldens: a quotation literal reaching a materialization
//! boundary (a struct field, an array element, a word output, or a branch
//! join) becomes a runtime `(code, env)` value, and `call`/`times` on an
//! erased quotation whose identity the checker cannot resolve emits an indirect
//! call. 7b (capturing closures): a captured *scalar* is snapshotted into the
//! env and admissible at every boundary; an *outer-rooted* reference (a `&T`
//! parameter) is admitted; a *frame-rooted* capture escaping the frame is a
//! past-owning-frame error; a captured quotation-typed name and a 2+-capture
//! escaping closure are deferred. A capturing literal at a direct `call` still
//! splices, unaffected. The branch join materializes each arm against the
//! enclosing declared output row (R11); the same literal in both arms stays a
//! splice.

use sooth::ast::WordBody;
use sooth::ir::{Instr, IrType};
use sooth::{check, lexer, parser};

mod common;

/// Compile and run `src`, returning stdout and the exit code. `name`
/// distinguishes the temp source per test (the goldens run in parallel).
fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
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

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

/// `lib/combinators.sth`'s `times`, inlined: the lowering-shape helpers below
/// run in process, where an `import:` line never resolves.
const TIMES_DEF: &str = ": times-helper inline ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
     | f | | to | | from |\n\
     from to < ~[ from f call from 1 + to f times-helper ] ~[ ] if ;\n\
     : times inline ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
     | f | | n | 0 n f times-helper ;\n";

#[test]
fn times_def_hand_copy_is_pinned_to_the_library() {
    common::assert_pinned_to_combinators_lib(TIMES_DEF, &[]);
}

/// Whether the lowered module emits at least one indirect call: the witness
/// that a `call` resolved to a runtime dispatch through a materialized value,
/// not a compile-time body splice.
fn emits_call_indirect(src: &str) -> bool {
    count_call_indirect(src) > 0
}

/// The total number of indirect calls in the lowered module: `times` over an
/// erased quotation must emit exactly one (in the loop body, not once per
/// iteration).
fn count_call_indirect(src: &str) -> usize {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = sooth::ir::lower(&module).expect("lower should succeed");
    ir.funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::CallIndirect(..)))
        .count()
}

/// The param list of the sole materialized quotation func (its symbol carries
/// `__quot`), for the 7b env-parameter shape assertions.
fn materialized_quot_params(src: &str) -> Vec<IrType> {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = sooth::ir::lower(&module).expect("lower should succeed");
    let mut mats: Vec<&sooth::ir::IrFunc> = ir
        .funcs
        .iter()
        .filter(|f| f.name.contains("__quot"))
        .collect();
    assert_eq!(mats.len(), 1, "expected exactly one materialized quotation");
    mats.remove(0).params.clone()
}

/// Every `Alloc` size (bytes) in the named lowered function -- the witness a
/// multi-capture materialization stack-allocates an env bundle (R16), distinct
/// from the one-word inline env a single capture keeps.
fn alloc_sizes(src: &str, func: &str) -> Vec<u32> {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = sooth::ir::lower(&module).expect("lower should succeed");
    ir.funcs
        .iter()
        .find(|f| f.name == func)
        .unwrap_or_else(|| panic!("function `{func}` should exist"))
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            Instr::Alloc(_, size, _) => Some(*size),
            _ => None,
        })
        .collect()
}

// -- T-field: a quotation stored in a struct field, called back out ----------

#[test]
fn quotation_stored_in_struct_field_compiles_and_calls() {
    let src = "type: Holder q [ i64 -- i64 ] ;\n\
               : main ( -- ) [ 1 + ] Holder Holder> 4 swap call . ;\n";
    let (stdout, code) = run_src("qfield", src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "the store/getter path must lower `call` to an indirect call"
    );
}

// -- T-array: two quotations in an array, each indirect-called ---------------

#[test]
fn quotation_in_array_element_indirect_calls() {
    // The seed element comes from a word returning an *erased* quotation (a
    // declared-effect context `fill` can intern an element type from); the
    // second element is a literal stored through the array's `&!` referent,
    // which carries the declared `[ i64 -- i64 ]` the store materializes it
    // against. A bare literal to `fill` has no effect context and is rejected.
    let src = ": one ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n\
               : main ( -- )\n\
               one 2 fill | a |\n\
               &!a 1 >usize &!> [ 2 + ] !\n\
               &a 0 &> @ 4 swap call .\n\
               &a 1 &> @ 4 swap call . ;\n";
    let (stdout, code) = run_src("qarray", src);
    assert_eq!(stdout, "5\n6\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "reading an element back and calling it must be an indirect call"
    );
}

// -- T-return: a word returning a quotation, indirect-called at the call site -

#[test]
fn quotation_returned_from_word_indirect_calls() {
    let src = ": mk ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n\
               : main ( -- ) mk 4 swap call . ;\n";
    let (stdout, code) = run_src("qreturn", src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "a returned quotation is erased at the call site, so `call` is indirect"
    );
}

// -- T-repoint (struct field): a captured scalar snapshots into the env -------

#[test]
fn capturing_scalar_stored_snapshots_into_env() {
    // 7b re-points 7a's `capturing_literal_stored_is_error_naming_7b`: `[ x + ]`
    // reads the enclosing scalar `x`, which is now snapshotted into the env
    // rather than rejected. Stored into a field, read back, and called with 4:
    // 4 + 10 = 14.
    let src = "type: Holder q [ i64 -- i64 ] ;\n\
               : main ( -- ) 10 | x | [ x + ] Holder Holder> 4 swap call . ;\n";
    let (stdout, code) = run_src("qcapfield", src);
    assert_eq!(stdout, "14\n");
    assert_eq!(code, 0);
}

// -- T-repoint (array element): a scalar snapshot stored through a reference --

#[test]
fn capturing_scalar_in_array_element_snapshots() {
    // The `!`/`+!` store boundary (an array element via reference), re-pointed:
    // `[ x + ]` snapshots `x = 10` into element 1, while element 0 keeps `one`'s
    // non-capturing seed; each reads its own env, proving coexistence. Element
    // 0: 4 + 1 = 5; element 1: 4 + 10 = 14.
    let src = ": one ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n\
               : main ( -- )\n\
               10 | x |\n\
               one 2 fill | a |\n\
               &!a 1 >usize &!> [ x + ] !\n\
               &a 0 &> @ 4 swap call .\n\
               &a 1 &> @ 4 swap call . ;\n";
    let (stdout, code) = run_src("qcaparray", src);
    assert_eq!(stdout, "5\n14\n");
    assert_eq!(code, 0);
}

// -- T-repoint (nested): a scalar captured through a *nested* quotation -------

#[test]
fn capturing_scalar_through_nested_quotation_snapshots() {
    // `x` is read inside a nested `[ x + ]`, not at the stored quotation's own
    // top level. The capture scan recurses into nested quotation bodies, so the
    // outer `[ [ x + ] call ]` snapshots `x` and stores admissibly: 4 + 10 = 14.
    let src = "type: Holder q [ i64 -- i64 ] ;\n\
               : main ( -- ) 10 | x | [ [ x + ] call ] Holder Holder> 4 swap call . ;\n";
    let (stdout, code) = run_src("qcapnested", src);
    assert_eq!(stdout, "14\n");
    assert_eq!(code, 0);
}

// -- T-repoint-bool/f32/f64/ints: every scalar type R15 case 1 admits --------
// -- round-trips correctly through the one-word env slot (B2 review fix) -----

#[test]
fn capturing_bool_scalar_snapshots_into_env() {
    // `x` is a captured `bool`, snapshotted into the env and read back. Before
    // the fix, reinterpreting the one-word env slot with a typed add-of-zero
    // segfaulted for `bool` (narrower than the env slot's own width, so the
    // add read garbage upper bytes); the fix round-trips through a scratch
    // slot at each type's own width instead.
    let src = "type: BoolHolder q [ -- bool ] ;\n\
               : main ( -- ) true | x | [ x ] BoolHolder BoolHolder> call . ;\n";
    let (stdout, code) = run_src("qcapbool", src);
    assert_eq!(stdout, "true\n");
    assert_eq!(code, 0);
}

#[test]
fn capturing_f32_scalar_snapshots_into_env() {
    // `x` is a captured `f32`; the QBE backend previously hard-failed here
    // (`invalid type for first operand ... in add`, a `d`/`l`-class mismatch)
    // since a float capture cannot share `Ptr`'s add-of-zero reinterpret.
    // 2.5 + 4.0 = 6.5, widened to `f64` by `.`.
    let src = "type: F32Holder q [ f32 -- f32 ] ;\n\
               : main ( -- ) 2.5 >f32 | x | [ x + ] F32Holder F32Holder> 4.0 >f32 swap call . ;\n";
    let (stdout, code) = run_src("qcapf32", src);
    assert_eq!(stdout, "6.5\n");
    assert_eq!(code, 0);
}

#[test]
fn capturing_f64_scalar_snapshots_into_env() {
    // The `f64` sibling of the `f32` case above, same backend failure mode.
    // 2.5 + 4.0 = 6.5.
    let src = "type: Holder q [ f64 -- f64 ] ;\n\
               : main ( -- ) 2.5 | x | [ x + ] Holder Holder> 4.0 swap call . ;\n";
    let (stdout, code) = run_src("qcapf64", src);
    assert_eq!(stdout, "6.5\n");
    assert_eq!(code, 0);
}

#[test]
fn capturing_i32_u32_and_usize_scalars_snapshot_into_env() {
    // Spot-check the narrower/unsigned integer captures the review flagged as
    // untested: `i32`, `u32`, `usize` each round-trip through the env
    // correctly (they already shared `Ptr`'s register class under the old
    // add-of-zero reinterpret, so these were not expected to regress, but were
    // never actually pinned by a golden). 10 + 4 = 14, 20 + 4 = 24, 30 + 4 = 34.
    let src = "type: I32Holder q [ i32 -- i32 ] ;\n\
               type: U32Holder q [ u32 -- u32 ] ;\n\
               type: UsizeHolder q [ usize -- usize ] ;\n\
               : main ( -- )\n\
               10 >i32 | a | [ a + ] I32Holder I32Holder> 4 >i32 swap call .\n\
               20 >u32 | b | [ b + ] U32Holder U32Holder> 4 >u32 swap call .\n\
               30 >usize | c | [ c + ] UsizeHolder UsizeHolder> 4 >usize swap call . ;\n";
    let (stdout, code) = run_src("qcapints", src);
    assert_eq!(stdout, "14\n24\n34\n");
    assert_eq!(code, 0);
}

// -- T-makeb: an outer-rooted reference capture (a `&T` parameter) is admitted -

#[test]
fn escaping_closure_over_param_ref_compiles_and_runs() {
    // `make-b`'s closure captures `r`, a `&[i64 4]` *parameter*: its referent
    // is rooted outside `make-b`'s frame (in `main`'s `a`, still live at the
    // call), so the escaping capture is admitted. The env holds the reference;
    // reading `r[0] = 5` and adding the input 4 gives 9.
    let src = ": make-b ( &[i64 4] -- [ i64 -- i64 ] ) | r | [ r 0 >usize &> @ + ] ;\n\
               : main ( -- ) 5 4 fill | a | &a make-b 4 swap call . ;\n";
    let (stdout, code) = run_src("qmakeb", src);
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
    // The materialized closure is erased at `call`, so dispatch through it is
    // an indirect call, not a compile-time splice.
    assert_eq!(
        count_call_indirect(src),
        1,
        "call through the captured closure is an indirect call"
    );
}

// -- T-env-inline: a one-capture materialized body takes one trailing Ptr env -

#[test]
fn materialized_single_capture_builds_inline_env() {
    // The inline single-capture env (R16/R17): `make-b`'s materialized body
    // gains exactly one trailing `Ptr` env parameter alongside its declared
    // `i64` input, and that pointer *is* the captured reference passed inline.
    // There is no `Alloc` bundle to assert against here -- bundle synthesis is
    // Phase 2's multi-capture path and does not exist yet, so asserting its
    // absence would be a placebo; the discriminating witness is the param
    // shape, which goes red if the env param (R17) is dropped.
    let src = ": make-b ( &[i64 4] -- [ i64 -- i64 ] ) | r | [ r 0 >usize &> @ + ] ;\n\
               : main ( -- ) 5 4 fill | a | &a make-b 4 swap call . ;\n";
    assert_eq!(
        materialized_quot_params(src),
        vec![
            IrType::Int {
                bits: 64,
                signed: true
            },
            IrType::Ptr
        ],
        "a one-capture materialized body has its declared input plus a Ptr env param"
    );
}

// -- T-makea: a frame-rooted capture escaping its owning frame is rejected ----

#[test]
fn escaping_closure_over_frame_local_is_past_owning_frame() {
    // `make-a`'s closure borrows `arr`, a `[i64 4]` bound *inside* `make-a`;
    // returning the closure would let it outlive `arr`'s storage. Unlike
    // `make-b`'s parameter, this is frame-rooted, so the escaping boundary is a
    // past-owning-frame error (R15/R24), asserted whole.
    let err = check_error(
        ": make-a ( -- [ i64 -- i64 ] ) 5 4 fill | arr | [ &arr 0 >usize &> @ + ] ;\n\
         : main ( -- ) make-a 4 swap call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure captures `arr`, a local of this frame, whose storage does not survive the return (line 1)"
    );
}

// -- T-makea-ref: a frame-rooted *borrow* (case 3, not case 2) is rejected ---

#[test]
fn escaping_closure_over_frame_local_borrow_is_past_owning_frame() {
    // `make-a` above captures `arr` itself (case 2, the aggregate). This
    // pins case 3: `r` is a *bound borrow* (`&arr | r |`) whose `owned_root`
    // is `arr`, a local of this frame -- the `owned_root`-in-scope test in
    // `classify_capture`'s `Type::Ref` arm, not the unconditional aggregate
    // arm. Rejected the same way, naming `r`.
    let err = check_error(
        ": make-a2 ( -- [ i64 -- i64 ] ) 5 4 fill | arr | &arr | r | [ r 0 >usize &> @ + ] ;\n\
         : main ( -- ) make-a2 4 swap call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure captures `r`, a local of this frame, whose storage does not survive the return (line 1)"
    );
}

// -- T-makea-param-store: a frame capture escaping via a store through a -----
// -- parameter-rooted reference is rejected (B1 review fix) -------------------

#[test]
fn frame_capture_escaping_via_store_through_param_ref_is_past_owning_frame() {
    // `install`'s closure borrows `r = &arr`, a *frame* local of `install`, and
    // is stored into an element of `tbl` reached through the `&![...]`
    // *parameter* reference. `tbl` itself is owned by `main`, outside
    // `install`'s frame, so the store boundary must be treated as escaping
    // (the same as returning the closure directly) even though the store
    // syntax looks like the in-frame R21 case. Before the fix this compiled
    // clean and stored a dangling reference into `tbl`.
    let err = check_error(
        ": seed ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n\
         : install ( &![ [ i64 -- i64 ] 2 ] -- )\n\
         5 4 fill | arr |\n\
         &arr | r |\n\
         0 >usize &!> [ r 0 >usize &> @ + ] ! ;\n\
         : main ( -- ) seed 2 fill | tbl | &!tbl install &tbl 0 &> @ 4 swap call . tbl drop ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure captures `r`, a local of this frame, whose storage does not survive the return (line 5)"
    );
}

// -- T-quot-cap-deferred: capturing a quotation-typed name is deferred --------

#[test]
fn capturing_quotation_typed_name_is_rejected_deferred() {
    // The outer `[ q call ]` captures `q`, itself a quotation local. A
    // quotation env slot is two words and needs a recursive surviving-set fold
    // (R15 case 4), so it is deferred at every boundary rather than admitted.
    let err = check_error(
        ": wrap ( -- [ i64 -- i64 ] ) [ 1 + ] | q | [ q call ] ;\n\
         : main ( -- ) wrap 4 swap call . ;\n",
    );
    assert_eq!(
        err,
        "error: capturing a quotation value by name is deferred (line 1)"
    );
}

// -- T-multi-esc: a 2+-capture escaping closure is deferred (single-word env) -

#[test]
fn multi_capture_escaping_closure_is_rejected_deferred() {
    // Phase 1's inline env holds one word; `[ x y + + ]` captures two scalars,
    // which needs a heap env (R18), deferred.
    let err = check_error(
        ": mk ( -- [ i64 -- i64 ] ) 10 | x | 20 | y | [ x y + + ] ;\n\
         : main ( -- ) mk 4 swap call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure may capture at most one reference (a heap env is deferred) (line 1)"
    );
}

// -- T-splice-cap: a capturing literal at a direct `call` still splices -------

#[test]
fn capturing_literal_spliced_still_works() {
    // Unchanged from the combinator slices: a capturing literal consumed by a
    // direct `call` is spliced in place (no materialization, no boundary), so
    // it reads the enclosing local and runs.
    let src = ": main ( -- ) 10 | x | 4 [ x + ] call . ;\n";
    let (stdout, code) = run_src("qsplice", src);
    assert_eq!(stdout, "14\n");
    assert_eq!(code, 0);
    assert!(
        !emits_call_indirect(src),
        "a spliced literal must not lower to an indirect call"
    );
}

// -- T-reg (R26): the env parameter (Phase 1) must not regress a spliced ------
// -- capturing literal into an indirect call, including through a combinator --

#[test]
fn capturing_literal_spliced_through_combinator_stays_splice() {
    // 7b makes a capturing literal legal at a materialization boundary, but a
    // force-inlined combinator (`times`, 6a's D2) still *splices* it: the body
    // `[ + x + ]` reads the enclosing local `x` in place per iteration, so
    // adding the env parameter to the materialized path (R17) must leave this
    // splice path bit-identical -- no materialization, no `CallIndirect`.
    // acc over i=0..4 of (acc + i + x=10): 10, 21, 33, 46, 60.
    let src = &format!("{TIMES_DEF}: main ( -- ) 10 | x | 0 5 ~[ + x + ] times . ;\n");
    let (stdout, code) = run_src("qcaptimes", src);
    assert_eq!(stdout, "60\n");
    assert_eq!(code, 0);
    assert!(
        !emits_call_indirect(src),
        "a capturing literal driven by a combinator is spliced, never indirect-called"
    );
}

// -- T-join: two differing quotation arms materialize against the declared ---
// -- output row and are indirect-called ---------------------------------------

#[test]
fn two_differing_quotation_arms_materialize_and_call() {
    // Each arm leaves a *different* literal; the join has no single body to
    // splice, so it materializes each against the word's declared
    // `[ i64 -- i64 ]` output (R11) and `call` at the site dispatches
    // indirectly through whichever aggregate the branch left.
    let src = ": pick ( bool -- [ i64 -- i64 ] ) ~[ [ 1 + ] ] ~[ [ 2 + ] ] if ;\n\
               : main ( -- ) true pick 4 swap call . false pick 4 swap call . ;\n";
    let (stdout, code) = run_src("qjoin", src);
    assert_eq!(stdout, "5\n6\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "a materialized branch-join quotation is indirect-called"
    );
}

// -- T-join-same: one literal in both arms stays a splice (no materialize) ----

#[test]
fn same_quotation_both_arms_still_splices() {
    // One literal bound before the `if`, named in both arms: the equal `Known`
    // ids forward the marker (no `Phi`, no materialization), so `call` after
    // the join splices exactly as today and emits no indirect call.
    let src = ": main ( -- ) [ 1 + ] | q | true ~[ q ] ~[ q ] if 5 swap call . ;\n";
    let (stdout, code) = run_src("qjoinsame", src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
    assert!(
        !emits_call_indirect(src),
        "the same literal in both arms is spliced, never indirect-called"
    );
}

// -- T-repoint-join: a scalar snapshot in one arm of a materializing join -----

#[test]
fn capturing_scalar_at_join_snapshots() {
    // A materializing join (the word declares a `[ i64 -- i64 ]` output) whose
    // `true` arm captures the scalar `x`. 7b re-points 7a's rejection: the arm
    // snapshots `x = 10` and both arms materialize (the `false` arm captures
    // nothing, a null env). `true`: 4 + 10 = 14; `false`: 4 + 2 = 6.
    let src = ": pick ( bool -- [ i64 -- i64 ] ) 10 | x | ~[ [ x + ] ] ~[ [ 2 + ] ] if ;\n\
               : main ( -- ) true pick 4 swap call . false pick 4 swap call . ;\n";
    let (stdout, code) = run_src("qcapjoin", src);
    assert_eq!(stdout, "14\n6\n");
    assert_eq!(code, 0);
}

// -- T-times: a loop over an erased quotation is one indirect call in a loop --

#[test]
fn loop_over_erased_quotation_emits_one_indirect_call() {
    // `acc` returns an *erased* quotation (a word-output materialization
    // boundary) and a self-tail combinator drives it. The checker accepts the
    // abstract `[ i64 i64 -- i64 ]` effect and lowering emits exactly one
    // `CallIndirect` inside the loop body (one indirect call per iteration,
    // no per-element call), summing 0+1+2+3+4 = 10. This does not run under a
    // stack limit -- the constant-stack guarantee itself is pinned by
    // `tests/phase4_slice10b.rs`'s `times_runs_one_million_iterations_in_constant_stack`.
    //
    // The driver is declared here rather than being `lib/combinators.sth`'s
    // `times`: 10b's `times` takes 10a's *inline-only* `~[ ... ]` parameter,
    // which an erased quotation cannot satisfy (it is rejected with `` `times`
    // expects a quotation `~[ i64 -- ]` here ``). So the erased-driver shape
    // this pins now needs a plain `[ ... ]` parameter, which is what the
    // intrinsic effectively had.
    let src = ": spin ( i64 i64 i64 [ i64 i64 -- i64 ] -- i64 )\n\
               | f | | to | | from |\n\
               from to < ~[ from f call from 1 + to f spin ] ~[ ] if ;\n\
               : acc ( -- [ i64 i64 -- i64 ] ) [ + ] ;\n\
               : main ( -- ) 0 0 5 acc spin . ;\n";
    let (stdout, code) = run_src("qtimes", src);
    assert_eq!(stdout, "10\n");
    assert_eq!(code, 0);
    assert_eq!(
        count_call_indirect(src),
        1,
        "an erased-quotation loop has exactly one indirect call, in the body"
    );
}

// -- T-dispatch: an array of same-frame capturing closures, indexed and called

#[test]
fn dispatch_table_of_capturing_closures_runs() {
    // Two closures, each capturing the shared frame borrow `r = &arr`, stored
    // into distinct array slots (in-frame boundaries, R21) and indexed-and-
    // `call`ed. Each reads a different element, so the table dispatches to the
    // same values the spliced form would: `arr = [7, 8]` -> `7`, `8`.
    let src = ": seed ( -- [ -- i64 ] ) [ 0 ] ;\n\
               : main ( -- )\n\
               7 2 fill | arr |\n\
               &!arr 1 >usize &!> 8 !\n\
               &arr | r |\n\
               seed 2 fill | tbl |\n\
               &!tbl 0 >usize &!> [ r 0 >usize &> @ ] !\n\
               &!tbl 1 >usize &!> [ r 1 >usize &> @ ] !\n\
               &tbl 0 &> @ call .\n\
               &tbl 1 &> @ call .\n\
               tbl drop\n\
               arr drop ;\n";
    let (stdout, code) = run_src("qdispatch", src);
    assert_eq!(stdout, "7\n8\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "a dispatch through an erased capturing closure is an indirect call"
    );
}

// -- T-lateread: a struct-stored closure observes a later mutation (D4) -------

#[test]
fn struct_stored_closure_observes_later_mutation() {
    // The closure captures the mutable borrow `r = &!arr`, is erased into a
    // struct field, then `arr` is mutated *through the same borrow* before the
    // `call`. R20 keeps `r` live across the store, so the mutation is admitted
    // and the late read observes it: the field is `9`, not the initial `0`.
    let src = "type: Holder q [ -- i64 ] ;\n\
               : main ( -- )\n\
               0 2 fill | arr |\n\
               &!arr | r |\n\
               [ r 0 >usize &!> @ ] Holder | h |\n\
               r 0 >usize &!> 9 !\n\
               h Holder> call .\n\
               arr drop ;\n";
    let (stdout, code) = run_src("qlateread", src);
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "a fetch-and-call through the stored closure is an indirect call"
    );
}

// -- T-lastuse: a captured referent killed before the call is past-last-use ---

#[test]
fn captured_reference_read_past_last_use_is_error() {
    // The closure captures `r = &!arr`, erased into a struct field (R20 keeps
    // `r` live to the `call`). A *separate* `&!arr` exclusive re-borrow before
    // the call would read the referent through a stale borrow, so it is
    // rejected: past-last-use (R24), naming `r`.
    let err = check_error(
        "type: Holder q [ -- i64 ] ;\n\
         : main ( -- )\n\
         0 2 fill | arr |\n\
         &!arr | r |\n\
         [ r 0 >usize &!> @ ] Holder | h |\n\
         &!arr 0 >usize &!> 9 !\n\
         h Holder> call .\n\
         arr drop ;\n",
    );
    assert_eq!(
        err,
        "error: a captured reference to `r` is read after its last use (line 6)"
    );
    // Contrast (probe P4 `lateread_known`): the *same* mutation with the
    // closure kept `Known` (never erased) is rejected as it is today -- a
    // conflicting-borrow error, not the new past-last-use wording.
    let known = check_error(
        ": main ( -- )\n\
         0 2 fill | arr |\n\
         &!arr | r |\n\
         [ r 0 >usize &!> @ ] | h |\n\
         &!arr 0 >usize &!> 9 !\n\
         h call .\n\
         arr drop ;\n",
    );
    assert!(
        known.contains("conflicts with a live borrow of `arr`"),
        "the Known-closure contrast keeps the conflicting-borrow wording: {known}"
    );
    assert!(
        !known.contains("read after its last use"),
        "the Known contrast must not reach the past-last-use path: {known}"
    );
}

// -- T-bundle: a two-capture in-frame closure stack-allocates its env bundle --

#[test]
fn materialized_multi_capture_builds_stack_bundle() {
    // Two captures (`ra`, `rb`) cannot ride the one-word inline env, so the
    // materialization stack-allocates a two-word bundle and points `env` at it
    // (R16). Witness: `main` gains exactly one 16-byte `Alloc` over the
    // otherwise-identical single-capture program (the quotation value and the
    // `Holder` shell are the same 16 bytes in both; only the bundle is new).
    // The run confirms both words are read back: `10 + 20 = 30`.
    let two = "type: Holder q [ -- i64 ] ;\n\
               : main ( -- )\n\
               10 1 fill | a |\n\
               20 1 fill | b |\n\
               &a | ra |\n\
               &b | rb |\n\
               [ ra 0 >usize &> @ rb 0 >usize &> @ + ] Holder | h |\n\
               h Holder> call .\n\
               a drop b drop ;\n";
    let one = "type: Holder q [ -- i64 ] ;\n\
               : main ( -- )\n\
               10 1 fill | a |\n\
               &a | ra |\n\
               [ ra 0 >usize &> @ ] Holder | h |\n\
               h Holder> call .\n\
               a drop ;\n";
    let (stdout, code) = run_src("qbundle", two);
    assert_eq!(stdout, "30\n");
    assert_eq!(code, 0);
    assert_eq!(
        materialized_quot_params(two),
        vec![IrType::Ptr],
        "the two-capture body still takes one trailing Ptr env param (the bundle pointer)"
    );
    let bundles = |src| {
        alloc_sizes(src, "main")
            .into_iter()
            .filter(|&s| s == 16)
            .count()
    };
    assert_eq!(
        bundles(two),
        bundles(one) + 1,
        "two captures add exactly one 16-byte env bundle alloc over one capture"
    );
}

// -- T-carrier: a frame capture escaping through a returned struct is caught --

#[test]
fn frame_capture_escaping_via_struct_is_past_owning_frame() {
    // The closure borrows `r = &arr`, a *frame* local of `make`, is stored into
    // a `Holder` and the `Holder` is returned. The frame capture would outlive
    // its storage, so the word-output escape guard (R22, walking the surviving
    // set the carrier holds -- `contains_reference` is blind to the env)
    // rejects it: past-owning-frame (R24), naming `r`.
    let err = check_error(
        "type: Holder q [ -- i64 ] ;\n\
         : make ( -- Holder )\n\
         0 2 fill | arr |\n\
         &arr | r |\n\
         [ r 0 >usize &> @ ] Holder ;\n\
         : main ( -- ) make Holder> call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure captures `r`, a local of this frame, whose storage does not survive the return (line 5)"
    );
}

// -- T-carrier-store/getter/cell: the surviving set (R19) must propagate -----
// -- through every value-producing path, not just the three the guard --------
// -- originally covered (constructor, store-onto-root-binding, if-join), so --
// -- R22/the store guard actually see a closure smuggled through a getter, --
// -- an array, or a heap cell (round-2 review fix) ---------------------------

#[test]
fn frame_capture_escaping_via_struct_carrier_stored_through_param_ref_is_past_owning_frame() {
    // `install`'s closure borrows `r = &arr`, a *frame* local of `install`, and
    // is wrapped into a `Holder` (an in-frame carrier, R21) before being
    // stored into `out`, a `&!Holder` *parameter* rooted outside `install`'s
    // frame. The store-boundary guard (da42294) only checked a still-literal
    // `Known` quotation before ever looking at escaping; `h` is no longer one
    // (the constructor already erased it), so the guard never ran at all
    // before this fix and the store compiled clean.
    let err = check_error(
        "type: Holder q [ -- i64 ] ;\n\
         : install ( &!Holder -- )\n\
         | out |\n\
         4242 4 fill | arr |\n\
         &arr | r |\n\
         [ r 0 >usize &> @ ] Holder | h |\n\
         out h !\n\
         arr drop ;\n\
         : main ( -- )\n\
         [ 7 ] Holder | box |\n\
         &!box install\n\
         box Holder> call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure captures `r`, a local of this frame, whose storage does not survive the return (line 7)"
    );
}

#[test]
fn frame_capture_escaping_via_struct_field_getter_return_is_past_owning_frame() {
    // `Holder>`'s field type is `Quotation`, not `is_aggregate`, so the
    // consuming getter falls to the generic env-based call dispatch, whose
    // constructor-output propagation gate required `is_aggregate` and so
    // dropped `h`'s surviving set even though `q` legitimately carries the
    // same closure onward. Before the fix `q`'s return skipped R22 entirely.
    let err = check_error(
        "type: Holder q [ -- i64 ] ;\n\
         : make ( -- [ -- i64 ] )\n\
         4242 4 fill | arr |\n\
         &arr | r |\n\
         [ r 0 >usize &> @ ] Holder | h |\n\
         h Holder> | q |\n\
         arr drop\n\
         q ;\n\
         : clobber ( -- ) 987654 8 fill | z | &z 0 >usize &> @ drop z drop ;\n\
         : main ( -- ) make | q | clobber q call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure captures `r`, a local of this frame, whose storage does not survive the return (line 8)"
    );
}

#[test]
fn frame_capture_escaping_via_heap_cell_return_is_past_owning_frame() {
    // `^` (owning-cell alloc) pushed a bare `Slot::computed`, dropping the
    // wrapped `Holder`'s surviving set; `c`'s return from `make` then skipped
    // R22 entirely, exactly as the struct/getter cases above did.
    let err = check_error(
        "type: Holder q [ -- i64 ] ;\n\
         : make ( -- ^Holder )\n\
         4242 4 fill | arr |\n\
         &arr | r |\n\
         [ r 0 >usize &> @ ] Holder ^ | c |\n\
         arr drop\n\
         c ;\n\
         : clobber ( -- )\n\
         987654 4 fill | z |\n\
         &z 0 >usize &> @ drop\n\
         z drop ;\n\
         : main ( -- )\n\
         make | c |\n\
         clobber\n\
         c ^> Holder> call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure captures `r`, a local of this frame, whose storage does not survive the return (line 7)"
    );
}

// -- T-propagate-getter/array/cell: the same three paths, but a genuinely ----
// -- in-frame closure (never escaping) still compiles and runs correctly -----
// -- after forwarding surviving through them (regression coverage) ----------

#[test]
fn same_frame_closure_through_struct_field_getter_runs() {
    let (stdout, code) = run_src(
        "getter-inframe",
        "type: Holder q [ -- i64 ] ;\n\
         : main ( -- )\n\
         4242 4 fill | arr |\n\
         &arr | r |\n\
         [ r 0 >usize &> @ ] Holder | h |\n\
         h Holder> call .\n\
         arr drop ;\n",
    );
    assert_eq!(stdout, "4242\n");
    assert_eq!(code, 0);
}

#[test]
fn same_frame_closure_through_array_element_access_runs() {
    let (stdout, code) = run_src(
        "array-inframe",
        "type: Holder q [ -- i64 ] ;\n\
         : main ( -- )\n\
         4242 4 fill | arr |\n\
         &arr | r |\n\
         [ r 0 >usize &> @ ] Holder 1 fill | tbl |\n\
         &tbl 0 >usize &> @ Holder> call .\n\
         arr drop tbl drop ;\n",
    );
    assert_eq!(stdout, "4242\n");
    assert_eq!(code, 0);
}

#[test]
fn same_frame_closure_through_heap_cell_runs() {
    let (stdout, code) = run_src(
        "cell-inframe",
        "type: Holder q [ -- i64 ] ;\n\
         : main ( -- )\n\
         4242 4 fill | arr |\n\
         &arr | r |\n\
         [ r 0 >usize &> @ ] Holder ^ | c |\n\
         c ^> Holder> call .\n\
         arr drop ;\n",
    );
    assert_eq!(stdout, "4242\n");
    assert_eq!(code, 0);
}

// -- T-bundle-carrier: a 2+-capture stack bundle escaping via a carrier is ----
// -- rejected even when no individual capture is frame-rooted (review fix) ---

#[test]
fn outer_rooted_bundle_escaping_via_carrier_is_rejected_deferred() {
    // `ra`/`rb` are both outer-rooted (`&[i64 2]` parameters, `deriv: None`),
    // so R22's frame-rooted walk sees no frame-rooted member and would have
    // wrongly admitted this before the fix. The closure still needs a
    // 2-capture stack bundle (R16), built in `make`'s frame; storing it into
    // `Holder` is an in-frame boundary (admitted, R21), but returning `Holder`
    // lets that bundle's own storage die at return regardless of who the
    // bundle's references point to.
    let err = check_error(
        "type: Holder q [ -- i64 ] ;\n\
         : make ( &[i64 2] &[i64 2] -- Holder )\n\
         | ra rb |\n\
         [ ra 0 >usize &> @ rb 0 >usize &> @ + ] Holder ;\n\
         : main ( -- )\n\
         10 2 fill | a |\n\
         20 2 fill | b |\n\
         &a &b make Holder> call .\n\
         a drop b drop ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure may capture at most one reference (a heap env is deferred) (line 4)"
    );
}

#[test]
fn scalar_and_ref_bundle_escaping_via_carrier_is_rejected_deferred() {
    // A scalar (`n`) plus an outer-rooted reference (`ra`) is still a
    // 2-*total*-capture bundle (R16), even though the surviving set has only
    // *one* member (`ra` -- a scalar snapshot is never a member, D4). Member
    // count alone cannot recover the bundle signal; this pins that the guard
    // tracks total capture count separately.
    let err = check_error(
        "type: Holder q [ -- i64 ] ;\n\
         : make ( &[i64 3] i64 -- Holder )\n\
         | ra n |\n\
         [ ra 0 >usize &> @ n + ] Holder ;\n\
         : main ( -- )\n\
         10 3 fill | a |\n\
         &a 5 make Holder> call .\n\
         a drop ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure may capture at most one reference (a heap env is deferred) (line 4)"
    );
}

#[test]
fn scalar_only_bundle_escaping_via_carrier_is_rejected_deferred() {
    // Both captures (`x`, `y`) are scalars, so the surviving set has *zero*
    // members (a scalar snapshot is never a member, D4). But two total captures
    // still allocate a 2-word stack bundle (R16) in `make`'s frame; that
    // bundle's own storage dies at return. Storing the closure into `Holder`
    // is an in-frame boundary (admitted, R21), and returning `Holder` must be
    // rejected. This is the empty-member edge: without the bundle marker
    // riding the interned set, the interned set would be `None` and R22 would
    // never fire, leaving a dangling stack bundle at runtime.
    let err = check_error(
        "type: Holder q [ -- i64 ] ;\n\
         : make ( -- Holder )\n\
         10 | x |\n\
         20 | y |\n\
         [ x y + ] Holder ;\n\
         : main ( -- ) make Holder> call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure may capture at most one reference (a heap env is deferred) (line 5)"
    );
}

// -- T-join: two capturing arms join, the union rides the erased slot ---------

#[test]
fn join_of_two_capturing_arms_unions_capture_sets() {
    // Two differing capturing literals joined and stored through a `&!q`
    // referent (an in-frame boundary that types the erased join). Both arms
    // capture `r = &arr`, so the merged closure reads it and, called, yields
    // `arr[0] = 10`; the join dispatch is indirect.
    let src = "type: Holder q [ -- i64 ] ;\n\
               : main ( -- )\n\
               10 2 fill | a |\n\
               &a | r |\n\
               [ 0 ] Holder | h |\n\
               &!h &!q true ~[ [ r 0 >usize &> @ ] ] ~[ [ r 1 >usize &> @ ] ] if !\n\
               h Holder> call .\n\
               a drop ;\n";
    let (stdout, code) = run_src("qjoinunion", src);
    assert_eq!(stdout, "10\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "the merged branch-join closure is indirect-called"
    );
}

// -- T-join-union: killing *either* arm's referent is past-last-use -----------

#[test]
fn join_capture_union_kills_either_arm_referent_is_past_last_use() {
    // Each arm captures a *distinct* borrow (`ra`, `rb`). The join interns the
    // union of both (R23), so killing *either* referent before the `call` is a
    // past-last-use. Both variants must fire: a "keep one arm's set" bug
    // survives killing the kept arm but is caught by killing the other.
    let program = |kill: &str| {
        format!(
            "type: Holder q [ -- i64 ] ;\n\
             : main ( -- )\n\
             10 2 fill | a |\n\
             20 2 fill | b |\n\
             &a | ra |\n\
             &b | rb |\n\
             [ 0 ] Holder | h |\n\
             &!h &!q true ~[ [ ra 0 >usize &> @ ] ] ~[ [ rb 0 >usize &> @ ] ] if !\n\
             &!{kill} 0 >usize &!> 9 !\n\
             h Holder> call .\n\
             a drop b drop ;\n"
        )
    };
    assert_eq!(
        check_error(&program("a")),
        "error: a captured reference to `ra` is read after its last use (line 9)"
    );
    assert_eq!(
        check_error(&program("b")),
        "error: a captured reference to `rb` is read after its last use (line 9)"
    );
}

// -- T-dogfood: examples/vm_table.sth matches examples/vm.sth byte-for-byte ---

#[test]
fn vm_table_dispatch_matches_clause_version() {
    // `examples/vm_table.sth` replaces `examples/vm.sth`'s clause-dispatched
    // `run` with a `decode` clause (the one unavoidable elimination, Q5) plus a
    // table of nine uniform, match-free `[ Vm -- Vm ]` handlers, indirect-called
    // by `dispatch`. Parity: same bytecode program, same result.
    let table_binary = common::build_example("examples/vm_table.sth");
    let table_stdout = std::process::Command::new(&table_binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("vm_table binary should run")
        .stdout;
    std::fs::remove_file(&table_binary).ok();

    let clause_binary = common::build_example("examples/vm.sth");
    let clause_stdout = std::process::Command::new(&clause_binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("vm binary should run")
        .stdout;
    std::fs::remove_file(&clause_binary).ok();

    assert_eq!(
        table_stdout, clause_stdout,
        "the table-dispatched VM must match the clause-dispatched one byte-for-byte"
    );

    let src = std::fs::read_to_string("examples/vm_table.sth").expect("read vm_table.sth");
    assert!(
        emits_call_indirect(&src),
        "the table dispatch path must emit at least one indirect call"
    );

    // M5: the table only proves the feature if the handlers are genuinely
    // match-free. Inlining a clause match back into a handler (defeating the
    // decode/execute split) must be caught here, not just by lowering.
    let tokens = lexer::lex(&src).expect("lexing vm_table.sth should succeed");
    let module = parser::parse(&tokens).expect("parsing vm_table.sth should succeed");
    let handler_names = [
        "h-push", "h-add", "h-sub", "h-mul", "h-load", "h-store", "h-jz", "h-jmp", "h-halt",
    ];
    for name in handler_names {
        let word = module
            .words
            .iter()
            .find(|w| w.name == name)
            .unwrap_or_else(|| panic!("vm_table.sth should define `{name}`"));
        assert!(
            matches!(word.body, WordBody::Terms { .. }),
            "handler `{name}` must be match-free (a term body, not a clause body)"
        );
    }
}

// -- T-dogfood: examples/capturing_dispatch.sth matches its hand-spliced twin -

#[test]
fn capturing_dispatch_matches_spliced_version() {
    // `examples/capturing_dispatch.sth` builds a dispatch table of two
    // same-frame capturing closures (T-dispatch's shape), each capturing a
    // shared borrow `r = &arr` and reading a different element; its
    // hand-spliced twin `examples/capturing_dispatch_hand.sth` reads the same
    // three elements directly, with no closure at all. Parity: same output.
    let closure_binary = common::build_example("examples/capturing_dispatch.sth");
    let closure_stdout = std::process::Command::new(&closure_binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("capturing_dispatch binary should run")
        .stdout;
    std::fs::remove_file(&closure_binary).ok();

    let hand_binary = common::build_example("examples/capturing_dispatch_hand.sth");
    let hand_stdout = std::process::Command::new(&hand_binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("capturing_dispatch_hand binary should run")
        .stdout;
    std::fs::remove_file(&hand_binary).ok();

    assert_eq!(
        closure_stdout, hand_stdout,
        "the capturing-closure dispatch table must match its hand-spliced twin byte-for-byte"
    );
    assert_eq!(closure_stdout, b"7\n8\n9\n");

    let src = std::fs::read_to_string("examples/capturing_dispatch.sth")
        .expect("read capturing_dispatch.sth");
    assert_eq!(
        count_call_indirect(&src),
        3,
        "each of the three table-stored closures dispatches through its own indirect call"
    );

    // The parity check above is the real witness that the env is non-null: a
    // closure reading through a null env could never observe `arr`'s distinct
    // stored elements, so matching the hand-spliced twin's `7\n8\n9\n` proves
    // each stored closure's reference capture is live and read.
}
