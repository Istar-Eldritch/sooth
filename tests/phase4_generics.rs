//! Phase 4 Slice 1 goldens. This phase covers the multi-output aggregate-return
//! ABI (R10, R11): a user word may declare two or more outputs, and calling one
//! now works instead of panicking the compiler.

use std::process::Command;

use sooth::{check, lexer, parser};

/// Phase 4 Slice 4, phase 2b/3: the sanctioned diagnostic helper (declared in
/// the spec's *Sanctioned edits*), copied from `tests/phase3_locals.rs`. This
/// file had no such helper and this slice's `times` typing has several
/// diagnostic negatives.
fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

/// The linear stand-in (a one-field struct with a `drop` overload). Two lines,
/// so a source prefixed with it shifts every line number up by 2.
const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy>tag . ;\n";

/// Compile and run `src`, returning its stdout and exit code. `name`
/// distinguishes the temp source (and so the emitted binary) per test, since
/// the goldens run in parallel in one process. `trace` sets the allocation
/// trace explicitly either way, so an ambient value can neither hide a trace
/// nor add one.
fn run_src(name: &str, src: &str, trace: bool) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
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
         true false swap . .\n\
         1 2 Vec2 dup Vec2>x . Vec2>y . ;\n",
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
        ": two ( -- i64 bool ) 1 true ;\n: main ( -- ) two . . ;\n",
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
         : spread ( -- i64 Vec2 bool ) 1 2 3 Vec2 true ;\n\
         : main ( -- ) spread . Vec2>y . . ;\n",
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
    assert_eq!(stdout, "alloc 8\n3\nfree 8\n7\n");
    assert_eq!(code, 0);
}

#[test]
fn copy_bounded_type_variable_word_runs_at_two_concrete_types() {
    // Criterion 3 (R1, R4–R7, R9, R14): a `'T: Copy` word `dup`s its variable
    // and is called at `i64` and `bool`. Each call site resolves to its own
    // monomorphized `IrFunc` through the instantiation table, so both
    // instantiations run and print both copies.
    let (stdout, code) = run_src(
        "dupit",
        ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n\
         : main ( -- ) 5 dupit . . true dupit . . ;\n",
        false,
    );
    assert_eq!(stdout, "5\n5\ntrue\ntrue\n");
    assert_eq!(code, 0);
}

#[test]
fn length_polymorphic_word_runs_over_two_array_lengths() {
    // Criterion 4 (R1, R5, R9): the recon-2 "unwritable" case, now written. A
    // length variable is opaque through `len`; monomorphization discharges it
    // to a concrete `N` per instantiation, so `alen` runs over both `[i64 4]`
    // and `[i64 8]` and prints each length.
    let (stdout, code) = run_src(
        "alen",
        ": alen ( [i64 'N] -- [i64 'N] usize ) len ;\n\
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
        ": dup2 ( ..s 'a: Copy 'b: Copy -- ..s 'a 'b 'a 'b ) over over ;\n\
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
        ": dup2 ( ..s 'a: Copy 'b: Copy -- ..s 'a 'b 'a 'b ) over over ;\n\
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
        ": split ( i64 -- i64 i64 ) dup 1 - ;\n\
         : main ( -- ) 5 split + . ;\n",
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
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
           | n a b |\n\
           n 0 = if b else\n\
             a Box>n .\n\
             n 1 - b a loop\n\
           end ;\n\
         : main ( -- ) 4 1 mk 2 mk loop Box>n . ;\n",
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
         : loop ( i64 Box -- Box ) | n b | n 0 = if b else n 1 - n mk loop end ;\n\
         : main ( -- ) 1000000 0 mk loop Box>n . ;\n";
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
         : loop ( i64 Box -- Box ) | n prev | n 0 = if prev else n 1 - prev loop end ;\n\
         : main ( -- ) 3 42 mk loop Box>n . ;\n",
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
         : drop ( Res -- ) | r | r Res>n 5000 + . ;\n\
         : mkres ( i64 -- Res ) | n | n Res ;\n\
         type: List | Nil | Cons v Res next ^List ;\n\
         : push-front ( List Res -- List ) | rest v | v rest ^ Cons ;\n\
         : build ( i64 List -- List )\n\
           | n acc |\n\
           n 0 = if acc else n 1 - acc n mkres push-front build end ;\n\
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
           n 0 = if b else\n\
             n 3 = if n mk else b end\n\
             | c |\n\
             n 1 - c loop\n\
           end ;\n\
         : main ( -- ) 5 0 mk loop Box>n . ;\n",
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
           n 0 = if prev else\n\
             n mk | cur |\n\
             prev Box>a .\n\
             n 1 - cur loop\n\
           end ;\n\
         : main ( -- ) 3 0 mk loop Box>a . ;\n",
        false,
    );
    assert_eq!(stdout, "0\n3\n2\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn array_carried_across_back_edge_is_not_aliased() {
    // Criterion 2 (R1-R4, R7): an `[i64 4]` via `4 fill`, re-produced each
    // iteration and read (through `&>`/`@`) the iteration after. Was `0 2 1`;
    // correct `0 3 2`.
    let (stdout, code) = run_src(
        "arrayalias",
        ": mkarr ( i64 -- [i64 4] ) 4 fill ;\n\
         : loop ( i64 [i64 4] -- [i64 4] )\n\
           | n prev |\n\
           n 0 = if prev else\n\
             n mkarr | cur |\n\
             &prev 0 &> @ .\n\
             n 1 - cur loop\n\
           end ;\n\
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
         : get ( E -- i64 ) | Wrap ;\n\
         : loop ( i64 E -- E )\n\
           | n prev |\n\
           n 0 = if prev else\n\
             n mk | cur |\n\
             prev get .\n\
             n 1 - cur loop\n\
           end ;\n\
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
         : drop ( Res -- ) | r | r Res>n 1000 + . ;\n\
         : mk ( i64 -- Res ) | n | n Res ;\n\
         : loop ( i64 Res -- Res )\n\
           | n prev |\n\
           n 0 = if prev else\n\
             n mk | cur |\n\
             prev drop\n\
             n 1 - cur loop\n\
           end ;\n\
         : main ( -- ) 3 0 mk loop drop ;\n",
        false,
    );
    assert_eq!(stdout, "1000\n1003\n1002\n1001\n");
    assert_eq!(code, 0);
}

#[test]
fn nested_projection_carried_across_back_edge_is_not_aliased() {
    // Criterion 7 (R4/D2): the back-edge `Vec2` arg is `s Segment>from`, an
    // interior pointer *into* the carried `Segment` stable slot (a distinct
    // `Value` from the slot), which read-before-write staging snapshots before
    // the `Segment` slot is overwritten. Was `99 0 2 1`; correct `99 0 3 2`.
    let (stdout, code) = run_src(
        "nestedproj",
        "type: Vec2 x i64 y i64 ;\n\
         type: Segment from Vec2 to Vec2 ;\n\
         : mkseg ( i64 -- Segment ) | n | n n Vec2 n 100 * n Vec2 Segment ;\n\
         : loop ( i64 Segment Vec2 -- Vec2 )\n\
           | n s v |\n\
           n 0 = if v else\n\
             v Vec2>x .\n\
             n 1 - n mkseg s Segment>from loop\n\
           end ;\n\
         : main ( -- ) 3 0 mkseg 99 99 Vec2 loop Vec2>x . ;\n",
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
           n 0 = if prev else\n\
             n n Vec2 | cur |\n\
             prev Vec2>x .\n\
             n 1 - cur loop\n\
           end ;\n\
         : main ( -- ) 3 0 0 Vec2 loop Vec2>x . ;\n",
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
           | n prev |\n\
           n 0 = if prev else\n\
             n 2 mod 0 = if\n\
               n mk | cur |\n\
               prev Box>n .\n\
               n 1 - cur loop\n\
             else\n\
               prev Box>n .\n\
               n 1 - prev loop\n\
             end\n\
           end ;\n\
         : main ( -- ) 4 0 mk loop Box>n . ;\n",
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
           n 0 = if u else\n\
             u drop\n\
             n .\n\
             n 1 - mku loop\n\
           end ;\n\
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
           | n a b c |\n\
           n 0 = if a else\n\
             a Box>n . b Box>n . c Box>n .\n\
             n 1 - c a b loop\n\
           end ;\n\
         : main ( -- ) 2 1 mk 2 mk 3 mk loop Box>n . ;\n",
        false,
    );
    assert_eq!(stdout, "1\n2\n3\n3\n1\n2\n2\n");
    assert_eq!(code, 0);
}

// --- Phase 4 Slice 4, phase 2a: the quotation marker + `call`-of-literal fusion.

#[test]
fn call_of_literal_quotation_fuses_and_runs() {
    // Criterion 2 (R6/R13): `[ + ] call` type-checks and lowers identically to
    // writing the body inline, so the fused `+` runs against the live stack.
    let (stdout, code) = run_src("call-literal", ": main ( -- ) 1 2 [ + ] call . ;\n", false);
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
        ": main ( -- ) [ + ] | q | 1 2 q call . ;\n",
        false,
    );
    assert_eq!(stdout, "3\n");
    assert_eq!(code, 0);
}

#[test]
fn quotation_body_reads_enclosing_local() {
    // Criterion 3b (R6 capture): the spliced body sees the current scope in
    // lexical extent, so `[ t + ]` reads the enclosing `t` with no capture
    // machinery.
    let (stdout, code) = run_src(
        "call-capture",
        ": main ( -- ) 7 | t | 1 [ t + ] call . ;\n",
        false,
    );
    assert_eq!(stdout, "8\n");
    assert_eq!(code, 0);
}

// --- Phase 4 Slice 4, phase 3: the `times` intrinsic and the constant-stack loop.

#[test]
fn times_loop_computes_the_index_sum() {
    // Criterion 4a (R14/R18): the headline value. `[ + ]` sums the index over
    // 0..1e6, so the loop runs exactly `count` iterations passing each index.
    let (stdout, code) = run_src(
        "times-sum",
        ": main ( -- ) 0 1000000 [ + ] times . ;\n",
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
        ": main ( -- ) 0 1000000 [ + ] times . ;\n",
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
        "type: Vec2 x i64 y i64 ;\n\
         : main ( -- ) 0 1000000 [ | i | i i Vec2 Vec2>x + ] times . ;\n",
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
        "type: Vec2 x i64 y i64 ;\n\
         : main ( -- ) 0 1000000 [ | i | i i Vec2 Vec2>x + ] times . ;\n",
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
        "type: Vec2 x i64 y i64 ;\n\
         : main ( -- ) 3 4 Vec2 0 1000000 [ drop over Vec2>x + ] times . drop ;\n",
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
    let (stdout, code) = run_src("times-zero", ": main ( -- ) 7 0 [ + ] times . ;\n", false);
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

#[test]
fn two_sequential_times_in_one_word_both_run() {
    // Criterion 15 (R15): two `times` in one word both run, and an aggregate
    // constructed *between* them prints its field -- witnessing that R15 restores
    // `entry_block` (not only `header`), or the aggregate's `Alloc` would hoist
    // into the first `times`'s dead entry block.
    let (stdout, code) = run_src(
        "times-sequential",
        "type: Vec2 x i64 y i64 ;\n\
         : main ( -- ) 0 10 [ + ] times . 5 6 Vec2 Vec2>x . 0 10 [ + ] times . ;\n",
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
        "{SPY_DEF}: main ( -- ) 5 Spy | s | 0 1000000 [ | i | s Spy>tag + ] times . ;\n"
    ));
    assert!(
        err.contains("a `times` body cannot consume `s`")
            && err.contains("the body runs more than once"),
        "R18a should name `s` and cite repeated disposal, got: {err}"
    );
}

#[test]
fn times_with_a_quotation_in_its_row_is_error() {
    // Criterion R18b (R18 whole-row guard): a quotation anywhere in the row --
    // not just the consumed top -- would reach `begin_loop`'s phi over a phantom,
    // so it is rejected (same wording family as R9/R11).
    let err = check_error(": main ( -- ) [ + ] 3 [ drop ] times ;\n");
    assert!(
        err.contains("`times`") && err.contains("cannot take a quotation as an operand"),
        "R18b should reject a quotation in the row, got: {err}"
    );
}

#[test]
fn times_body_changing_the_row_is_error() {
    // Criterion R18c (D6 row-effect equality): `[ + 1 ]` leaves the row one
    // deeper than it received, so the body's net effect is not identity.
    let err = check_error(": main ( -- ) 0 1000000 [ + 1 ] times . ;\n");
    assert!(
        err.contains("`times` body must leave the row unchanged"),
        "R18c should fire the row-effect error, got: {err}"
    );
}

#[test]
fn times_nested_in_a_loop_is_rejected() {
    // Criterion N (R18): a `times` nested in a loop is rejected in the checker
    // with a line number -- both a `times` inside another `times` body (splice
    // depth) and a `times` inside a self-tail word (`has_self_tail_call`).
    let inner = check_error(": main ( -- ) 0 10 [ | i | 0 5 [ + ] times + ] times . ;\n");
    assert!(
        inner.contains("a `times` cannot be nested in a loop yet") && inner.contains("(line 1)"),
        "N (times-in-times) should reject with a line number, got: {inner}"
    );
    let self_tail = check_error(
        ": loop ( i64 -- i64 ) | n | n 0 = if 0 else n 1 - 0 5 [ + ] times drop n 1 - loop end ;\n\
         : main ( -- ) 3 loop . ;\n",
    );
    assert!(
        self_tail.contains("a `times` cannot be nested in a loop yet")
            && self_tail.contains("(line 1)"),
        "N (times in self-tail word) should reject with a line number, got: {self_tail}"
    );
}

// --- Phase 4 Slice 4, phase 4: dogfood + docs.

#[test]
fn times_example_matches_hand_threaded_countdown() {
    // Criterion 7: `examples/times.sth` (`0 1000000 [ 1 + + ] times .`) builds and
    // prints the same total as `examples/countdown.sth`'s hand-threaded
    // self-recursive sum 1..1e6, demonstrating parity between the internal loop
    // primitive and the slice 6 self-tail-call -> loop transform rather than a
    // value off by 1e6.
    let binary = sooth::driver::build(std::path::Path::new("examples/times.sth"))
        .expect("build should succeed");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        "500000500000\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
