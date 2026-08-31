//! Phase 4 Slice 1 goldens. This phase covers the multi-output aggregate-return
//! ABI (R10, R11): a user word may declare two or more outputs, and calling one
//! now works instead of panicking the compiler.

use std::process::Command;

use sooth::{check, lexer, test_support};

mod common;

/// Phase 4 Slice 4, phase 2b/3: the sanctioned diagnostic helper (declared in
/// the spec's *Sanctioned edits*), copied from `tests/phase3_locals.rs`. This
/// file had no such helper and this slice's `times` typing has several
/// diagnostic negatives.
fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

/// The linear stand-in (a one-field struct with a `drop` overload). Two lines,
/// so a source prefixed with it shifts every line number up by 2.
const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | s Spy> drop ;\n";

/// `lib/combinators.sth`'s `times`, inlined: `check_error` runs the checker in
/// process, where an `import:` line never resolves.
const TIMES_DEF: &str = ": times-helper inline ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
     | f | | to | | from |\n\
     from to lt ~[ from f call from 1 add to f times-helper ] ~[ ] if ;\n\
     : times inline ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
     | f | | n | 0 n f times-helper ;\n";

#[test]
fn times_def_hand_copy_is_pinned_to_the_library() {
    common::assert_pinned_to_combinators_lib(TIMES_DEF, &[]);
}

/// An `import:` line for the committed combinator library by *absolute* path,
/// so a temp source built under `temp_dir()` resolves it regardless of cwd.
fn combinators_import(qualifier: &str) -> String {
    format!(
        "import: \"{}/lib/core/combinators.sth\" {qualifier} ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Compile and run `src`, returning its stdout and exit code. `name`
/// distinguishes the temp source (and so the emitted binary) per test, since
/// the goldens run in parallel in one process. `trace` sets the allocation
/// trace explicitly either way, so an ambient value can neither hide a trace
/// nor add one.
fn run_src(name: &str, src: &str, trace: bool) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    let mut cmd = Command::new(&binary);
    match trace {
        true => cmd.env(sooth::ir::TRACE_ALLOC_ENV, "1"),
        false => cmd.env_remove(sooth::ir::TRACE_ALLOC_ENV),
    };
    let output = cmd.output().expect("binary should run");
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

#[test]
fn core_shuffles_are_polymorphic_over_i64_bool_and_a_struct() {
    // Criterion 1 (S3, R2): `dup`/`swap` were already type-transparent before
    // this slice (`check_shuffle` moves `Slot`s verbatim; the `lower_call`
    // shuffle arms dispatch on runtime `value_type`), so this pins that they
    // run correctly over `i64`, `bool`, and a struct in one program, without
    // acquiring any `PolySig`/monomorphization machinery.
    let (stdout, code) = run_src(
        "shuffles",
        "type: Vec2 x i64 y i64 ;\n\
         : main ( -- )\n\
         5 dup . .\n\
         True False swap . .\n\
         1 2 Vec2 &x @ . &y @ . drop ;\n",
        false,
    );
    assert_eq!(stdout, "5\n5\ntrue\nfalse\n1\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn two_output_word_call_pushes_both_results() {
    // Criterion 2 (R10, R11): the recon-3 reproduction. `: pair` checked and
    // lowered before this slice, but calling it dropped the second result and
    // panicked lowering on the next consumer of the value never pushed.
    let (stdout, code) = run_src(
        "pair",
        ": pair ( i64 -- i64 i64 ) dup ;\n: main ( -- ) 5 pair . . ;\n",
        false,
    );
    assert_eq!(stdout, "5\n5\n");
    assert_eq!(code, 0);
}

#[test]
fn two_output_word_outputs_arrive_deepest_first() {
    // R10/R11: the bundle preserves output order — the leftmost declared
    // output is deepest, so the two prints come off top-first.
    let (stdout, code) = run_src(
        "order",
        ": two ( -- i64 Bool ) 1 True ;\n: main ( -- ) two . . ;\n",
        false,
    );
    assert_eq!(stdout, "true\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn three_output_word_with_an_aggregate_output_runs() {
    // R10/R11: the ABI is count-agnostic (which is what lets a resolved row
    // variable ride it later), and a struct output travels in the bundle by
    // value like any other field.
    let (stdout, code) = run_src(
        "three",
        "type: Vec2 x i64 y i64 ;\n\
         : spread ( -- i64 Vec2 Bool ) 1 2 3 Vec2 True ;\n\
         : main ( -- ) spread . &y @ . drop . ;\n",
        false,
    );
    assert_eq!(stdout, "true\n3\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn two_output_word_with_a_linear_output_frees_its_cell_exactly_once() {
    // Criterion 10 (R10, R11, key risk 1): the bundle for `( -- ^i64 i64 )`
    // folds linear, since its first field is an owning cell. The caller's
    // unpack moves that cell out, and the bundle itself carries no destructor,
    // so the trace shows exactly one alloc and one free — no double free, no
    // leak.
    let (stdout, code) = run_src(
        "cellpair",
        ": cell-and-tag ( -- ^i64 i64 ) 7 ^ 3 ;\n: main ( -- ) cell-and-tag . ^> . ;\n",
        true,
    );
    assert_eq!(stdout, "3\n7\nalloc 8\nfree 8\n");
    assert_eq!(code, 0);
}

#[test]
fn copy_bounded_type_variable_word_runs_at_two_concrete_types() {
    // Criterion 3 (R1, R4–R7, R9, R14): a `'T: Copy` word `dup`s its variable
    // and is called at `i64` and `Bool`. Each call site resolves to its own
    // monomorphized `IrFunc` through the instantiation table, so both
    // instantiations run and print both copies.
    let (stdout, code) = run_src(
        "dupit",
        ": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n\
         : main ( -- ) 5 dupit . . True dupit . . ;\n",
        false,
    );
    assert_eq!(stdout, "5\n5\ntrue\ntrue\n");
    assert_eq!(code, 0);
}

#[test]
fn length_polymorphic_word_runs_over_two_array_lengths() {
    // Criterion 4 (R1, R5, R9): the recon-2 "unwritable" case, now written. A
    // length variable is opaque through `len`; monomorphization discharges it
    // to a concrete `N` per instantiation, so `alen` runs over both `array[i64 4]`
    // and `array[i64 8]` and prints each length.
    let (stdout, code) = run_src(
        "alen",
        ": alen ( array[i64 'N] -- array[i64 'N] usize ) len ;\n\
         : main ( -- ) 5 4 fill alen . drop 5 8 fill alen . drop ;\n",
        false,
    );
    assert_eq!(stdout, "4\n8\n");
    assert_eq!(code, 0);
}

#[test]
fn row_variable_word_expands_to_a_multi_output_bundle_and_runs() {
    // Criterion 5 (R1, R5, R9, R10, R11, R14): a row-variable word passes its
    // deeper stack through untouched and `over over`s the two `Copy`
    // variables. Its resolved instantiation has four concrete outputs, so it
    // lowers through the same pack/unpack bundle a fixed multi-output word
    // does (D4, one mechanism): `1 2 dup2` leaves `1 2 1 2`, printed top-first.
    let (stdout, code) = run_src(
        "dup2",
        ": dup2 ['a: Copy 'b: Copy] ( ..s 'a 'b -- ..s 'a 'b 'a 'b ) over over ;\n\
         : main ( -- ) 1 2 dup2 . . . . ;\n",
        false,
    );
    assert_eq!(stdout, "2\n1\n2\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn row_variable_passes_a_non_empty_below_stack_through_in_order() {
    // R5/R9: the row variable is a below-stack marker, so values pushed before
    // the fixed inputs must survive untouched and in order beneath the word's
    // outputs. Here `..s` binds `9 8` (not the empty stack the sibling golden
    // exercises): `9 8 1 2 dup2` leaves `9 8 1 2 1 2`, so printing top-first
    // yields the two duplicated outputs, then the two originals, then `8 9`.
    let (stdout, code) = run_src(
        "dup2_row",
        ": dup2 ['a: Copy 'b: Copy] ( ..s 'a 'b -- ..s 'a 'b 'a 'b ) over over ;\n\
         : main ( -- ) 9 8 1 2 dup2 . . . . . . ;\n",
        false,
    );
    assert_eq!(stdout, "2\n1\n2\n1\n8\n9\n");
    assert_eq!(code, 0);
}

#[test]
fn two_output_word_result_feeds_another_call() {
    // R11: the unpacked values are ordinary stack values, so they flow into
    // the next word exactly as any other operands do.
    let (stdout, code) = run_src(
        "chain",
        ": split ( i64 -- i64 i64 ) dup 1 sub ;\n\
         : main ( -- ) 5 split add . ;\n",
        false,
    );
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
}

#[test]
fn max_over_the_integer_tower_prints_the_larger_operand() {
    // Criterion 6 (R12): `max` over `i64`, `u8`, and `usize`, each printing
    // the larger operand of its pair.
    let (stdout, code) = run_src(
        "maxint",
        ": main ( -- )\n\
         3 5 max .\n\
         7 >u8 2 >u8 max .\n\
         9 >usize 20 >usize max . ;\n",
        false,
    );
    assert_eq!(stdout, "5\n7\n20\n");
    assert_eq!(code, 0);
}

#[test]
fn max_total_over_floats_prints_the_total_ordered_larger() {
    // Criterion 7 (R13): `max-total` over two `f64` and two `f32`. The first
    // two lines are positive pairs; the negative and mixed-sign pairs exercise
    // the sign-set branch of the `total_cmp` key, where a raw unsigned compare
    // of the bit patterns would pick the wrong operand without the transform.
    let (stdout, code) = run_src(
        "maxtotal",
        ": main ( -- )\n\
         1.5 2.5 max-total .\n\
         3.5 >f32 1.5 >f32 max-total .\n\
         -3.0 -5.0 max-total .\n\
         -1.0 2.0 max-total . ;\n",
        false,
    );
    assert_eq!(stdout, "2.5\n3.5\n-3\n2\n");
    assert_eq!(code, 0);
}

// Phase 4 Slice 3: loop-carried aggregate aliasing and the back-edge copy.
// Phase 1 lands the regression guards that already pass on the current tree,
// before the lowering change (R1-R4) that could break them.

/// Like `run_src`, but bounds the *stack* rather than the address space (1 MB
/// via `ulimit -s`) and is signal-aware: a missed hoist is a `SIGSEGV`, which
/// `run_src`'s `.code().expect(...)` would panic on rather than report.
fn run_stack_bounded_src(name: &str, src: &str) -> Option<i32> {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    std::fs::remove_file(&path).ok();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("ulimit -s 1024 && exec \"{}\"", binary.display()))
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .status()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    status.code()
}

#[test]
fn two_aggregates_swapped_across_back_edge_stay_correct() {
    // Criterion 5 (R4/D2 regression guard): neither carried `Box` is
    // re-produced in the loop, so the back-edge is a pure swap (`b a`).
    // Green today; the parallel-copy shape the fix's staging must not corrupt.
    let (stdout, code) = run_src(
        "swap",
        "type: Box n i64 ;\n\
         : mk ( i64 -- Box ) | n | n Box ;\n\
         : loop ( i64 Box Box -- Box )\n\
           | k a b |\n\
           k 0 eq ~[ b ] ~[\n\
             &a &n @ .\n\
             k 1 sub b a loop\n\
           ] if ;\n\
         : main ( -- ) 4 1 mk 2 mk loop &n @ . drop ;\n",
        false,
    );
    assert_eq!(stdout, "1\n2\n1\n2\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn aggregate_carried_loop_runs_in_constant_stack() {
    // Criterion 6 (R9): the carried `Box` is re-produced every iteration and
    // is not forwarded-in-place, so it stages under the fix. Green today
    // (the existing entry-hoisted alloc), guarding that the fix introduces
    // no per-iteration stack bump.
    let src = "type: Box n i64 ;\n\
         : mk ( i64 -- Box ) | n | n Box ;\n\
         : loop ( i64 Box -- Box ) | n b | n 0 eq ~[ b ] ~[ n 1 sub n mk loop ] if ;\n\
         : main ( -- ) 1000000 0 mk loop &n @ . drop ;\n";
    assert_eq!(
        run_stack_bounded_src("aggloop", src),
        Some(0),
        "a fixed-count aggregate-carrying loop should run in constant stack, not overflow it"
    );
}

#[test]
fn forwarded_aggregate_reads_its_seeded_value() {
    // Criterion 9 (R3/R4): `prev` is carried unchanged (never re-produced), so
    // it is forwarded in place; today that means its phi arm is the seed
    // param on the entry edge. Non-zero seed (42) so a fix that skips the
    // entry-arm init blit reliably reads as not-42, not a QBE-zeroed alloc
    // that happens to also read 0.
    let (stdout, code) = run_src(
        "seeded",
        "type: Box n i64 ;\n\
         : mk ( i64 -- Box ) | n | n Box ;\n\
         : loop ( i64 Box -- Box ) | n prev | n 0 eq ~[ prev ] ~[ n 1 sub prev loop ] if ;\n\
         : main ( -- ) 3 42 mk loop &n @ . drop ;\n",
        false,
    );
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

#[test]
fn recursive_type_destructor_disposes_right_contents() {
    // Criterion 10 (R5, R1a behavioural regression guard): a three-node
    // `List` whose payload is a `drop`-overloaded `Res`, disposed by the
    // fused iterative destructor (Phase 3 slice 8b). Green today, and stays
    // green whether or not R1a's gate is later missed (the destructor loop
    // is correct today by its own read-then-overwrite ordering); the gate
    // itself is pinned structurally, not by this behavioural golden.
    let (stdout, code) = run_src(
        "listdrop",
        "type: Res n i64 ;\n\
         : drop ( Res -- ) | r | r Res> 5000 add . ;\n\
         : mkres ( i64 -- Res ) | n | n Res ;\n\
         type: List | Nil | Cons v Res next ^List ;\n\
         : push-front ( List Res -- List ) | rest v | v rest ^ Cons ;\n\
         : build ( i64 List -- List )\n\
           | n acc |\n\
           n 0 eq ~[ acc ] ~[ n 1 sub acc n mkres push-front build ] if ;\n\
         : main ( -- ) 3 Nil build drop ;\n",
        false,
    );
    assert_eq!(stdout, "5001\n5002\n5003\n");
    assert_eq!(code, 0);
}

#[test]
fn join_phi_over_carried_aggregate_survives() {
    // Criterion 11 (R2 scoping guard): the inner `if` is not in tail
    // position, so both arms fall through to a join phi over the carried
    // `Box` (one arm re-produces, the other forwards); the merged value is
    // then carried to the tail call as a normal arg. Green today, guarding
    // that the fix's no-header-phi rule (R2) stays scoped to the loop header
    // and does not suppress this ordinary join phi.
    let (stdout, code) = run_src(
        "joinphi",
        "type: Box n i64 ;\n\
         : mk ( i64 -- Box ) | n | n Box ;\n\
         : loop ( i64 Box -- Box )\n\
           | n b |\n\
           n 0 eq ~[ b ] ~[\n\
             n 3 eq ~[ n mk ] ~[ b ] if\n\
             | c |\n\
             n 1 sub c loop\n\
           ] if ;\n\
         : main ( -- ) 5 0 mk loop &n @ . drop ;\n",
        false,
    );
    assert_eq!(stdout, "3\n");
    assert_eq!(code, 0);
}

// Phase 2 fix-witnesses: red on the current tree, green only with the R1-R4
// lowering fix. Each carried aggregate is re-produced (or projected) before the
// prior iteration's value is read, so pre-fix it aliases the reused storage.

#[test]
fn struct_carried_across_back_edge_is_not_aliased() {
    // Criterion 1 (R1-R4, R7): a two-field struct re-produced each iteration
    // and read the iteration after. Was `0 2 1 1`; correct `0 3 2 1`.
    let (stdout, code) = run_src(
        "structalias",
        "type: Box a i64 b i64 ;\n\
         : mk ( i64 -- Box ) | n | n n Box ;\n\
         : loop ( i64 Box -- Box )\n\
           | n prev |\n\
           n 0 eq ~[ prev ] ~[\n\
             n mk | cur |\n\
             &prev &a @ .\n\
             n 1 sub cur loop\n\
           ] if ;\n\
         : main ( -- ) 3 0 mk loop &a @ . drop ;\n",
        false,
    );
    assert_eq!(stdout, "0\n3\n2\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn array_carried_across_back_edge_is_not_aliased() {
    // Criterion 2 (R1-R4, R7): an `array[i64 4]` via `4 fill`, re-produced each
    // iteration and read (through `&>`/`@`) the iteration after. Was `0 2 1`;
    // correct `0 3 2`.
    let (stdout, code) = run_src(
        "arrayalias",
        ": mkarr ( i64 -- array[i64 4] ) 4 fill ;\n\
         : loop ( i64 array[i64 4] -- array[i64 4] )\n\
           | n prev |\n\
           n 0 eq ~[ prev ] ~[\n\
             n mkarr | cur |\n\
             &prev 0 &> @ .\n\
             n 1 sub cur loop\n\
           ] if ;\n\
         : main ( -- ) 3 0 mkarr loop drop ;\n",
        false,
    );
    assert_eq!(stdout, "0\n3\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn enum_carried_across_back_edge_is_not_aliased() {
    // Criterion 3 (R1-R4, R7): a single-variant enum re-produced each
    // iteration and read the iteration after. Not by-construction: it
    // reproduces the identical live miscompile. Was `0 2 1 1`; correct
    // `0 3 2 1`.
    let (stdout, code) = run_src(
        "enumalias",
        "type: E | Wrap v i64 ;\n\
         : mk ( i64 -- E ) | n | n Wrap ;\n\
         : get ( E -- i64 ) ~[ ( Wrap ) Wrap> ] E? ;\n\
         : loop ( i64 E -- E )\n\
           | n prev |\n\
           n 0 eq ~[ prev ] ~[\n\
             n mk | cur |\n\
             prev get .\n\
             n 1 sub cur loop\n\
           ] if ;\n\
         : main ( -- ) 3 0 mk loop get . ;\n",
        false,
    );
    assert_eq!(stdout, "0\n3\n2\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn destructor_carried_across_back_edge_disposes_right_contents() {
    // Criterion 4 (R4, R5): the resource-safety bar. A `drop`-overloaded `Res`
    // disposed one iteration late; disposal stays exactly-once *and* disposes
    // the right contents. Was `1000 1002 1001 1001` (a double-dispose by
    // content and a leak); correct `1000 1003 1002 1001`.
    let (stdout, code) = run_src(
        "dtoralias",
        "type: Res n i64 ;\n\
         : drop ( Res -- ) | r | r Res> 1000 add . ;\n\
         : mk ( i64 -- Res ) | n | n Res ;\n\
         : loop ( i64 Res -- Res )\n\
           | n prev |\n\
           n 0 eq ~[ prev ] ~[\n\
             n mk | cur |\n\
             prev drop\n\
             n 1 sub cur loop\n\
           ] if ;\n\
         : main ( -- ) 3 0 mk loop drop ;\n",
        false,
    );
    assert_eq!(stdout, "1000\n1003\n1002\n1001\n");
    assert_eq!(code, 0);
}

#[test]
fn nested_projection_carried_across_back_edge_is_not_aliased() {
    // Criterion 7 (R4/D2): the back-edge `Vec2` arg is `&s &from @`, an
    // interior pointer *into* the carried `Segment` stable slot (a distinct
    // `Value` from the slot), which read-before-write staging snapshots before
    // the `Segment` slot is overwritten. Was `99 0 2 1`; correct `99 0 3 2`.
    let (stdout, code) = run_src(
        "nestedproj",
        "type: Vec2 x i64 y i64 ;\n\
         type: Segment from Vec2 to Vec2 ;\n\
         : mkseg ( i64 -- Segment ) | n | n n Vec2 n 100 mul n Vec2 Segment ;\n\
         : loop ( i64 Segment Vec2 -- Vec2 )\n\
           | n s v |\n\
           n 0 eq ~[ v ] ~[\n\
             &v &x @ .\n\
             n 1 sub n mkseg &s &from @ loop\n\
           ] if ;\n\
         : main ( -- ) 3 0 mkseg 99 99 Vec2 loop &x @ . drop ;\n",
        false,
    );
    assert_eq!(stdout, "99\n0\n3\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn inline_constructed_aggregate_carried_across_back_edge_is_not_aliased() {
    // Criterion 8 (R1-R4): a carried `Vec2` built inline each iteration, with
    // no producer call, read the iteration after. The storage-reuse witness
    // that the cause is reused entry-hoisted storage, not the return ABI. Was
    // `0 2 1 1`; correct `0 3 2 1`.
    let (stdout, code) = run_src(
        "inlinealias",
        "type: Vec2 x i64 y i64 ;\n\
         : loop ( i64 Vec2 -- Vec2 )\n\
           | n prev |\n\
           n 0 eq ~[ prev ] ~[\n\
             n n Vec2 | cur |\n\
             &prev &x @ .\n\
             n 1 sub cur loop\n\
           ] if ;\n\
         : main ( -- ) 3 0 0 Vec2 loop &x @ . drop ;\n",
        false,
    );
    assert_eq!(stdout, "0\n3\n2\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn back_edges_disagreeing_on_a_carried_slot_stage_independently() {
    // R4/D2 per-predecessor partition: `finalize_loop` decides forward-vs-stage
    // once *per back-edge*, so a word with two distinct tail self-calls into the
    // same header can forward the carried `Box` in place on one edge and stage
    // it on the other. Both `loop` calls target the same header; the even arm
    // re-produces (`n mk` -> `cur`, a fresh slot -> staged) while the odd arm
    // carries `prev` unchanged (exactly the stable slot -> forwarded in place),
    // and each re-produce reads `prev` *after* `mk` has reused the storage. The
    // single-back-edge goldens above never exercise two edges disagreeing on
    // one slot. Pre-fix the forwarded `prev` was an interior pointer into the
    // reused `mk` slot that a later re-produce clobbered: was `0 4 2 2 2`;
    // correct `0 4 4 2 2`.
    let (stdout, code) = run_src(
        "twobackedge",
        "type: Box n i64 ;\n\
         : mk ( i64 -- Box ) | n | n Box ;\n\
         : loop ( i64 Box -- Box )\n\
           | k prev |\n\
           k 0 eq ~[ prev ] ~[\n\
             k 2 mod 0 eq ~[\n\
               k mk | cur |\n\
               &prev &n @ .\n\
               k 1 sub cur loop\n\
             ] ~[\n\
               &prev &n @ .\n\
               k 1 sub prev loop\n\
             ] if\n\
           ] if ;\n\
         : main ( -- ) 4 0 mk loop &n @ . drop ;\n",
        false,
    );
    assert_eq!(stdout, "0\n4\n4\n2\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn zero_size_aggregate_carried_across_back_edge_runs_correctly() {
    // A zero-field struct is a zero-size aggregate, so `begin_loop`'s init blit
    // (`size > 0`) and `finalize_loop`'s Pass-2 staging (`size == 0` skip) both
    // guard against emitting a zero-byte `Blit`. Re-producing the `Unit` each
    // iteration (a fresh `Unit`, not the carried stable slot) drives the staged
    // branch rather than forward-in-place, so it is the Pass-2 `size == 0` skip
    // that elides the blit, not the identity check. The carried `Unit` holds no
    // bytes to witness; the scalar counter carries alongside it, so correct
    // countdown output confirms the guarded paths run rather than trapping on a
    // zero-size blit.
    let (stdout, code) = run_src(
        "zerosize",
        "type: Unit ;\n\
         : mku ( -- Unit ) Unit ;\n\
         : loop ( i64 Unit -- Unit )\n\
           | n u |\n\
           n 0 eq ~[ u ] ~[\n\
             u drop\n\
             n .\n\
             n 1 sub mku loop\n\
           ] if ;\n\
         : main ( -- ) 3 Unit loop drop ;\n",
        false,
    );
    assert_eq!(stdout, "3\n2\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn three_aggregates_rotated_across_back_edge_stay_correct() {
    // R4/D2 cycle-length generality: the back-edge is a three-cycle (`c a b`),
    // so slot `a` takes `c`, `b` takes `a`, and `c` takes `b` -- every slot
    // stages and none forwards in place. Read-before-write staging must
    // snapshot all three sources before any store lands, exactly as it does for
    // the two-slot swap; a staging keyed to the swap case (or that lost a slot)
    // would corrupt the rotation. The per-iteration triples witness a clean
    // rotation: `1 2 3` becomes `3 1 2` (each position shifted, not a pairwise
    // swap and no dropped slot), and the base returns the twice-rotated `a`
    // (`2`).
    let (stdout, code) = run_src(
        "rotate3",
        "type: Box n i64 ;\n\
         : mk ( i64 -- Box ) | n | n Box ;\n\
         : loop ( i64 Box Box Box -- Box )\n\
           | k a b c |\n\
           k 0 eq ~[ a ] ~[\n\
             &a &n @ . &b &n @ . &c &n @ .\n\
             k 1 sub c a b loop\n\
           ] if ;\n\
         : main ( -- ) 2 1 mk 2 mk 3 mk loop &n @ . drop ;\n",
        false,
    );
    assert_eq!(stdout, "1\n2\n3\n3\n1\n2\n2\n");
    assert_eq!(code, 0);
}

// --- Phase 4 Slice 4, phase 2a: the quotation marker + `call`-of-literal fusion.

#[test]
fn call_of_literal_quotation_fuses_and_runs() {
    // Criterion 2 (R6/R13): `[ add ] call` type-checks and lowers identically to
    // writing the body inline, so the fused `add` runs against the live stack.
    let (stdout, code) = run_src(
        "call-literal",
        ": main ( -- ) 1 2 [ add ] call . ;\n",
        false,
    );
    assert_eq!(stdout, "3\n");
    assert_eq!(code, 0);
}

#[test]
fn quotation_forwarded_through_bind_still_calls() {
    // Criterion 3 (R4's `Binding` forwarding): a quotation bound to a local and
    // read back is still `call`-able, so the marker survives the bind's fresh
    // `Slot` reconstruction.
    let (stdout, code) = run_src(
        "call-bind",
        ": main ( -- ) [ add ] | q | 1 2 q call . ;\n",
        false,
    );
    assert_eq!(stdout, "3\n");
    assert_eq!(code, 0);
}

#[test]
fn quotation_body_reads_enclosing_local() {
    // Criterion 3b (R6 capture): the spliced body sees the current scope in
    // lexical extent, so `[ t add ]` reads the enclosing `t` with no capture
    // machinery.
    let (stdout, code) = run_src(
        "call-capture",
        ": main ( -- ) 7 | t | 1 [ t add ] call . ;\n",
        false,
    );
    assert_eq!(stdout, "8\n");
    assert_eq!(code, 0);
}

// --- Phase 4 Slice 4, phase 2b: the default-deny diagnostics. A quotation is a
// `Cstr` placeholder, so a missed guard is a *silent accept*, not a mismatch;
// each negative here pins that a specific consumer rejects it with the right,
// site-naming wording (R7-R11, R19). The completeness of the operand family is
// the load-bearing `quotation_as_operand_is_rejected_at_every_audited_site`
// checker unit; these goldens pin the surface behaviour per criterion.

#[test]
fn different_quotations_at_a_join_are_error() {
    // R7: two `if` arms each leaving a *different* quotation merge at the join;
    // reject there (not at consumption), since the branch lowering would build
    // a `Phi` over two phantoms even when the merge is only `drop`ped. Slice
    // 10c: the arms are quotation literals passed to a `lib/` word, so the
    // join is located at the first arm rather than at `branch` itself, which
    // sits in library source the user did not write.
    let err = check_error(": main ( -- ) True ~[ [ 1 add ] ] ~[ [ 1 sub ] ] if drop ;\n");
    assert!(
        err.contains("these two branches leave different quotations") && err.contains("line 1"),
        "R7 should fire at the join, got: {err}"
    );
}

#[test]
fn quotation_versus_value_at_a_join_is_error() {
    // R7n: one arm leaves a quotation, the other a *real* `cstr`. Their `ty`s
    // are equal (the placeholder is `Cstr`), so the ordinary branch mismatch
    // never fires; the join guard's second phrasing catches it.
    let err = check_error(": main ( -- ) True ~[ [ 1 add ] ] ~[ \"x\" cstr ] if drop ;\n");
    assert!(
        err.contains("one branch of the `if`")
            && err.contains("leaves a quotation and the other does not"),
        "R7n should fire the second phrasing, got: {err}"
    );
}

#[test]
fn quotation_stored_in_array_by_fill_is_error() {
    // R8f: a quotation element to `fill` would become a runtime array value.
    // The guard sits strictly above `contains_reference` (R4).
    let err = check_error(": main ( -- ) [ add ] 8 fill drop ;\n");
    assert!(
        err.contains("a quotation cannot be stored"),
        "R8f should reject at `fill`, got: {err}"
    );
}

#[test]
fn quotation_stored_through_a_reference_is_error() {
    // R8r: storing a quotation into a `&!cstr` referent makes
    // `match_slot(Cstr, Cstr)` return `Exact` -- a *silent accept* -- so the
    // guard must sit strictly above `match_slot`. This proves guard placement,
    // not merely wording.
    let err = check_error(
        "type: Box s cstr ;\n\
         : main ( -- ) \"hi\" cstr Box | b | &!b &!s [ add ] ! b drop ;\n",
    );
    assert!(
        err.contains("a quotation cannot be stored"),
        "R8r should reject the stored value before `match_slot`, got: {err}"
    );
}

#[test]
fn quotation_passed_to_user_word_is_error() {
    // R9: only `call`/`times` accept a quotation this slice; a user `:` word
    // rejects before ordinary unification, naming the word.
    let err = check_error(": foo ( i64 -- i64 ) ;\n: main ( -- ) [ add ] foo drop ;\n");
    assert!(
        err.contains("a quotation cannot be passed to `foo`"),
        "R9 should name the word, got: {err}"
    );
}

#[test]
fn quotation_passed_to_polymorphic_word_is_error() {
    // R9p: `check_poly_call` reads only `.ty`, so a quotation *succeeds*
    // unification and binds `'T` to the placeholder without the guard. Reject
    // before `unify_poly_input`.
    let err = check_error(
        ": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n: main ( -- ) [ add ] dupit drop drop ;\n",
    );
    assert!(
        err.contains("a quotation cannot be passed to `dupit`"),
        "R9p should name the polymorphic word, got: {err}"
    );
}

// R5p's blanket "a quotation literal in a polymorphic body is rejected"
// rule is retired by P7 slice 3b: a literal is admitted and consumed by an
// in-body eliminator. What survives of it -- the rejection of a quotation
// that is *not* consumed there -- lives in `tests/phase7_slice3b.rs`.

#[test]
fn quotation_left_on_stack_is_output_error() {
    // R10: the output count matches (one output, one quotation), so the new
    // branch must beat the ordinary type mismatch that would leak the `Cstr`
    // placeholder.
    let err = check_error(": f ( -- i64 ) [ add ] ;\n");
    assert!(
        err.contains("`f`")
            && err.contains("leaves a quotation on the stack")
            && err.contains("declared output"),
        "R10 should be the dedicated output diagnostic, got: {err}"
    );
}

#[test]
fn quotation_as_operator_operand_is_error() {
    // R11: a quotation as an operator operand rejects, naming `add`.
    let err = check_error(": main ( -- ) 1 [ add ] add ;\n");
    assert!(
        err.contains("`add`") && err.contains("cannot take a quotation as an operand"),
        "R11 should name `add`, got: {err}"
    );
}

#[test]
fn quotation_as_if_condition_is_error() {
    // R11if: a quotation in the condition position is rejected naming the
    // word, never leaking a `bool` mismatch. Slice 10c: `if` is a `lib/` word,
    // so the rejection is the combinator argument guard's rather than the
    // retired `if` arm's own -- same site, same guarantee, different wording.
    let err = check_error(": main ( -- ) ~[ add ] ~[ 1 drop ] ~[ 2 drop ] if ;\n");
    assert!(
        err.contains("`if`") && err.contains("a quotation cannot be passed to"),
        "R11if should name `if`, not a Bool mismatch, got: {err}"
    );
    assert!(
        !err.contains("Bool"),
        "R11if must not leak a Bool mismatch, got: {err}"
    );
}

#[test]
fn quotation_dropped_is_a_pure_pop() {
    // R11drop: the one legal unguarded consumer. `drop` of a compile-time-only
    // marker discards it with nothing to dispose, so this runs and prints `1`.
    let (stdout, code) = run_src("drop-quot", ": main ( -- ) 1 [ add ] drop . ;\n", false);
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

#[test]
fn two_calls_of_a_binding_quotation_body_both_run() {
    // R6br1 (blocker 1): the `call` splice must save-and-truncate `self.locals`,
    // or the first body's `| x |` leaves a stale entry the second body's bind
    // reads front-first. Without the fix this prints `4\n4\n`.
    let (stdout, code) = run_src(
        "call-rebind",
        ": main ( -- ) 2 [ | x | x x add ] call . 3 [ | x | x x add ] call . ;\n",
        false,
    );
    assert_eq!(stdout, "4\n6\n");
    assert_eq!(code, 0);
}

#[test]
fn linear_bound_inside_a_quotation_body_is_error() {
    // R6br2: a linear value bound inside the body and left unconsumed is caught
    // only because the splice is bracketed by `leave_block`; the `call` is where
    // the body's scope ends, so the unconsumed `s` is rejected there.
    let err = check_error(&format!(
        "{SPY_DEF}: main ( -- ) [ 5 Spy | s | 42 ] call drop ;\n"
    ));
    assert!(
        err.contains("linear value `s`") && err.contains("never consumed"),
        "R6br2 should reject the unconsumed linear local, got: {err}"
    );
}

#[test]
fn times_body_constructing_a_quotation_into_the_row_is_error() {
    // Blocker 2: a body that consumes a real value and constructs a quotation
    // into that slot leaves a phantom in the *output* row. With `times` an
    // ordinary library word (10b), the intrinsic's own output-row guard is
    // gone; the rejection now comes from the branch-join guard inside the
    // spliced `times-helper`, whose tail `if` sees a quotation on one arm only.
    let err = check_error(&format!(
        "{TIMES_DEF}: main ( -- ) \"x\" cstr 0 ~[ drop drop [ add ] ] times drop ;\n"
    ));
    assert!(
        err.contains("leaves a quotation and the other does not")
            && err.contains("a quotation cannot be a runtime value"),
        "blocker 2 should reject the body-output row quotation, got: {err}"
    );
}

// --- Phase 4 Slice 4, phase 3: the `times` intrinsic and the constant-stack loop.

#[test]
fn times_loop_computes_the_index_sum() {
    // Criterion 4a (R14/R18): the headline value. `[ add ]` sums the index over
    // 0..1e6, so the loop runs exactly `count` iterations passing each index.
    let (stdout, code) = run_src(
        "times-sum",
        &format!(
            "{}: main ( -- ) 0 1000000 ~[ add ] times . ;\n",
            combinators_import("c | times |")
        ),
        false,
    );
    assert_eq!(stdout, "499999500000\n");
    assert_eq!(code, 0);
}

#[test]
fn times_loop_runs_in_constant_stack() {
    // Criterion 4b: a cheap regression tripwire (not the R17 witness). 4a's
    // source emits no `Alloc`, so no plausible lowering fails this while passing
    // criterion 6; the real R17 witness is 5b plus criterion 6's entry-block
    // `Alloc` assertion.
    let code = run_stack_bounded_src(
        "times-sum-bounded",
        &format!(
            "{}: main ( -- ) 0 1000000 ~[ add ] times . ;\n",
            combinators_import("c | times |")
        ),
    );
    assert_eq!(code, Some(0));
}

#[test]
fn times_body_constructing_aggregate_computes_expected() {
    // Criterion 5a (R17): the body constructs a 16-byte `Vec2` each iteration.
    // Without the entry-block hoist that is ~16 MB against the 1 MB bound, so
    // this value golden and 5b together witness the constant-stack guarantee.
    let (stdout, code) = run_src(
        "times-aggregate",
        &format!(
            "{}type: Vec2 x i64 y i64 ;\n\
             : main ( -- ) 0 1000000 ~[ | i | i i Vec2 &x @ swap drop add ] times . ;\n",
            combinators_import("c | times |")
        ),
        false,
    );
    assert_eq!(stdout, "499999500000\n");
    assert_eq!(code, 0);
}

#[test]
fn times_body_constructing_aggregate_runs_in_constant_stack() {
    // Criterion 5b (R17 end-to-end backstop): 5a's source runs under the 1 MB
    // bound only because every per-iteration `Vec2` `Alloc` hoists into the
    // entry block (one reused slot), not the body block.
    let code = run_stack_bounded_src(
        "times-aggregate-bounded",
        &format!(
            "{}type: Vec2 x i64 y i64 ;\n\
             : main ( -- ) 0 1000000 ~[ | i | i i Vec2 &x @ swap drop add ] times . ;\n",
            combinators_import("c | times |")
        ),
    );
    assert_eq!(code, Some(0));
}

#[test]
fn times_carrying_an_aggregate_through_the_row_runs() {
    // Criterion 5c (slice 3 `CarriedSlot::Aggregate` staging from a non-self-tail
    // driver): a `Vec2` rides the row through 1e6 iterations, staged on its
    // stable slot on the back-edge, never re-allocated.
    let (stdout, code) = run_src(
        "times-carry-aggregate",
        &format!(
            "{}type: Vec2 x i64 y i64 ;\n\
             : main ( -- ) 3 4 Vec2 0 1000000 ~[ drop over &x @ swap drop add ] times . drop ;\n",
            combinators_import("c | times |")
        ),
        false,
    );
    assert_eq!(stdout, "3000000\n");
    assert_eq!(code, 0);
}

#[test]
fn times_zero_trip_yields_seed_row() {
    // Criterion 5z: a zero count runs the body zero times, so the row leaves the
    // loop untouched (the seed `7`), and a non-zero, non-index seed proves the
    // exit reads the carried row, not the index.
    let (stdout, code) = run_src(
        "times-zero",
        &format!(
            "{}: main ( -- ) 7 0 ~[ add ] times . ;\n",
            combinators_import("c | times |")
        ),
        false,
    );
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

#[test]
fn two_sequential_times_in_one_word_both_run() {
    // Criterion 15 (R15): two `times` in one word both run, and an aggregate
    // constructed *between* them prints its field -- witnessing that R15 restores
    // `entry_block` (not only `header`), or the aggregate's `Alloc` would hoist
    // into the first `times`'s dead entry block. Both bodies bind `| i |` so the
    // `times` half of the locals-splice leak is witnessed too: without the
    // save-and-truncate, the first body's stale `i` would shadow the second's.
    let (stdout, code) = run_src(
        "times-sequential",
        &format!(
            "{}type: Vec2 x i64 y i64 ;\n\
             : main ( -- ) 0 10 ~[ | i | i add ] times . 5 6 Vec2 &x @ . drop 0 10 ~[ | i | i add ] times . ;\n",
            combinators_import("c | times |")
        ),
        false,
    );
    assert_eq!(stdout, "45\n5\n45\n");
    assert_eq!(code, 0);
}

#[test]
fn times_body_consuming_a_linear_local_is_error() {
    // Criterion R18a (R18 move-state identity): the body is spliced once but
    // runs N times, so consuming the outer linear `s` would dispose it N times.
    // Named `s`, with the "body runs more than once" wording.
    let err = check_error(&format!(
        "{TIMES_DEF}{SPY_DEF}: main ( -- ) 5 Spy | s | 0 1000000 ~[ | i | i s drop add ] times drop ;\n"
    ));
    assert!(
        err.contains(
            "the quotation passed to `times` consumes the enclosing local `s`, which is linear"
        ) && err.contains("may only read a `Copy` enclosing local by value"),
        "R18a should name `s` and cite the capture-admission rule, got: {err}"
    );
}

#[test]
fn quotation_left_as_a_declared_output_is_error() {
    // Not R18b-specific: the intrinsic's whole-row guard that used to reject
    // this shape went with it in 10b, and no general guard replaced it. What
    // rejects this program is the ordinary outputs check on `main` -- a
    // quotation left on the stack doesn't match the declared (empty) output
    // row. A row quotation that is disposed rather than left on the stack is
    // *not* rejected at all -- it reaches the backend as an invalid phi. That
    // hole predates 10b (it reproduces on any user-declared row combinator,
    // `my-times` included) and is recorded in `docs/roadmap/P4/slice10b-spec.md`;
    // this golden pins only what does reject.
    let err = check_error(&format!(
        "{TIMES_DEF}: main ( -- ) [ add ] 3 ~[ drop ] times ;\n"
    ));
    assert!(
        err.contains("leaves a quotation on the stack"),
        "an unconsumed quotation should reject against the declared outputs, got: {err}"
    );
}

#[test]
fn times_body_changing_the_row_is_error() {
    // Criterion R18c (D6 row-effect equality): `array[ add 1 ]` leaves the row one
    // deeper than it received, so the body's net effect is not identity.
    let err = check_error(&format!(
        "{TIMES_DEF}: main ( -- ) 0 1000000 ~[ add 1 ] times drop ;\n"
    ));
    assert!(
        err.contains("the quotation passed to `times` was declared `~[ i64 -- ]`")
            && err.contains("but its body has effect `[ i64 -- i64 ]`"),
        "R18c should reject the body whose effect is not the declared row identity, got: {err}"
    );
}

// 6d retired `times_nested_in_a_loop_is_rejected`: the nested-loop rejection
// (R18/R14b) is gone, so both its programs now compile and run. The
// nesting-matrix goldens live in `tests/phase4_combinators.rs` (criteria 1-8).

// --- Phase 4 Slice 4, phase 4: dogfood + docs.

#[test]
fn times_example_matches_hand_threaded_countdown() {
    // Criterion 7: `examples/times.sth` (`0 1000000 [ 1 add add ] times .`) builds and
    // prints the same total as `examples/countdown.sth`'s hand-threaded
    // self-recursive sum 1..1e6, demonstrating parity between the internal loop
    // primitive and the slice 6 self-tail-call -> loop transform rather than a
    // value off by 1e6.
    let binary = common::build_example("examples/times.sth");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        "500000500000\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

// --- Phase 4 Slice 6e, phase 2: end-to-end goldens for `if` in a polymorphic
// body, plus the nested-`if` dogfood example.

#[test]
fn poly_mymax_runs_at_i64_and_f64() {
    // T9: `mymax`'s body branches on a raw comparison, instantiated at `i64`
    // and `f64` in one program, `inline` because `if` is an ordinary word
    // taking two quotation literals and a non-spliced polymorphic body
    // rejects a quotation outright (S3o); the branch runs through the
    // ordinary splice now.
    //
    // P7.S3s (R7): `inline` words can now declare a `Bound::User` variable
    // (the gate that formerly rejected it is gone, replaced by the
    // per-splice trait-call resolution path), and `Ord` is one now. This
    // fixture keeps its own point -- an `inline` generic body splicing `if`
    // correctly -- by not exercising that path: it drops the `Ord` bound
    // and the library `gt` call it would need, comparing through the raw
    // `ugt` intrinsic wrapped in the same `[ True ] [ False ] branch`
    // construction `gt` itself is built over, so the body stays free of
    // trait dispatch while still producing the `Bool` its own `if` consumes.
    let (stdout, code) = run_src(
        "poly-mymax",
        ": mymax inline ['T: Copy] ( 'T 'T -- 'T ) over over ugt [ True ] [ False ] branch ~[ drop ] ~[ swap drop ] if ;\n\
         : main ( -- ) 3 7 mymax . 3.0 7.0 mymax . ;\n",
        false,
    );
    assert_eq!(stdout, "7\n7\n");
    assert_eq!(code, 0);
}

#[test]
fn poly_choose_runs_at_i64_and_f64() {
    // T10: `choose`'s unbounded `'T` body (the acceptance witness rewritten
    // from T1) instantiated at `i64` and `f64`, each printing the kept
    // operand.
    let (stdout, code) = run_src(
        "poly-choose",
        ": choose inline ( 'T 'T Bool -- 'T ) | a b flag | flag ~[ a b drop ] ~[ b a drop ] if ;\n\
         : main ( -- ) 1 2 True choose . 1.0 2.0 False choose . ;\n",
        false,
    );
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn poly_nested_if_dogfood_runs() {
    // T11: `examples/poly_if.sth`'s `mymax3` nests an `if` inside an `if`
    // arm (D4's proof), instantiated at `i64` and `f64`.
    let binary = common::build_example("examples/poly_if.sth");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        "9\n9\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
