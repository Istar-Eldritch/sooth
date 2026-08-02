//! Phase 4 Slice 1 goldens. This phase covers the multi-output aggregate-return
//! ABI (R10, R11): a user word may declare two or more outputs, and calling one
//! now works instead of panicking the compiler.

use std::process::Command;

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
