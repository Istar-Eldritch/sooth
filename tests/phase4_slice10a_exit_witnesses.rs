//! Slice 10a, phase 7: the exit witnesses (R15-R19). `my-times`, the recon-4
//! user-space quotation loop the whole slice exists to make writable, compiles
//! beside the untouched `times` intrinsic, sums correctly, runs a million
//! iterations in constant stack, nests, carries an aggregate without
//! aliasing, and its row grounding is pinned to lose provenance (a borrow can
//! be substituted for an unrelated one of the same referent type). R19 pins
//! that nothing else moved: the intrinsic, `while`, the corpus, and the
//! library are unchanged against the base commit this slice branches from.

static NEXT_TEMP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn run_src(name: &str, src: &str) -> (String, i32) {
    let id = NEXT_TEMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}-{id}.sth", std::process::id()));
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

fn build_binary(name: &str, src: &str) -> std::path::PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}-{id}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    std::fs::remove_file(&path).ok();
    binary
}

/// Mirrors `tests/phase4_combinators.rs`'s helper: run `binary` under
/// `ulimit -s {limit_kb}` (KB), returning the exit code (`None` on a signal
/// death, e.g. a stack-overflow `SIGSEGV`) and trimmed stdout.
fn run_at_stack_limit(binary: &std::path::Path, limit_kb: u32) -> (Option<i32>, String) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -s {limit_kb} && exec \"{}\"",
            binary.display()
        ))
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    )
}

/// The recon-4 `my-times`, spelled with `..s i64 i64 ~[ ..s i64 -- ..s ]`:
/// `from`/`to` are two separate `i64` counters, not a typo for `times`'s own
/// single count.
const MY_TIMES: &str = ": my-times ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
     | f | | to | | from |\n\
     from to < if\n\
       from f call\n\
       from 1 + to f my-times\n\
     else\n\
     end ;\n";

// -- R15: user-space `my-times` compiles beside the untouched intrinsic -----

#[test]
fn my_times_compiles_beside_the_untouched_intrinsic_and_sums() {
    // Both the intrinsic `times` and the user-space `my-times` are called in
    // the same program, so this pins that 10a added a *writable signature*
    // rather than replacing the intrinsic: `times` still exists, unchanged
    // shape (`( ..s i64 -- ..s )`). `0 0 5 [ + ] my-times` sums 0+1+2+3+4 =
    // 10; `0 3 [ 1 + + ] times` sums (0+1)+(1+1)+(2+1) = 6.
    let src =
        format!("{MY_TIMES}: main ( -- )\n  0 0 5 [ + ] my-times .\n  0 3 [ 1 + + ] times . ;\n");
    let (stdout, code) = run_src("my-times-sum", &src);
    assert_eq!(stdout, "10\n6\n");
    assert_eq!(code, 0);
}

#[test]
fn my_times_runs_one_million_iterations_in_constant_stack() {
    // R15: `run_at_stack_limit` at `ulimit -s 1024`, the same constant-stack
    // witness `three_deep_times_nesting_runs_in_constant_stack`
    // (`tests/phase4_combinators.rs:1403`) uses for the intrinsic. A
    // per-iteration `Call` (no TCO) would overflow this stack long before 1M
    // rounds.
    let src = format!("{MY_TIMES}: main ( -- ) 0 0 1000000 [ drop 1 + ] my-times . ;\n");
    let binary = build_binary("my-times-1m", &src);
    let (code, out) = run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        (code, out),
        (Some(0), "1000000".to_string()),
        "1M iterations of a user-space `~`-typed self-tail loop must run to completion in constant stack"
    );
}

// -- R16: grounding is type-only, so provenance does not survive the row ----

#[test]
fn row_grounding_accepts_a_borrow_of_an_unrelated_place_of_the_same_type() {
    // R16 (load-bearing): row grounding is type equality over the region, not
    // a proof it was "restored unchanged". `apply-with-v`'s body `[ swap drop
    // ]` drops the *incoming* row borrow (of `a`) and leaves the *fixed*
    // parameter borrow (of `b`) as the new row -- an unrelated referent of the
    // same `&!V` type. `Slot::computed` drops `deriv`, so nothing in the
    // grounding mechanism notices or objects; the printed field (9, `b`'s)
    // proves the substitution actually took effect at runtime, not merely
    // that the check let it through.
    let src = "type: V x i64 ;\n\
               : apply-with-v ( ..s &!V ~[ ..s &!V -- ..s ] -- ..s )\n\
               | f | f call ;\n\
               : main ( -- )\n\
               0 V | a |\n\
               9 V | b |\n\
               &!a &!b [ swap drop ] apply-with-v\n\
               &!V>x @ .\n\
               a drop b drop ;\n";
    let (stdout, code) = run_src("row-borrow-substitution", src);
    assert_eq!(
        stdout, "9\n",
        "the surviving borrow must be `b`'s (9), proving the substitution, not `a`'s (0)"
    );
    assert_eq!(code, 0);
}

// -- R17: aggregate carried across the row, per-iteration data dependence ---

#[test]
fn my_times_carries_an_aggregate_without_aliasing() {
    // R17: an aggregate (`Acc`) rides the row across 5 iterations, each
    // reading the previous iteration's fields to compute the next (`new_x =
    // x0 + i`, `new_y = y0 + x0 + i`) -- per-iteration data dependence, so a
    // stale or aliased blit of the carried struct (the slice-3 aliasing
    // class) would surface as a wrong number, not a crash. Hand-traced:
    // (x,y) goes (0,0)->(0,0)->(1,1)->(3,4)->(6,10)->(10,20).
    let src = format!(
        "type: Acc x i64 y i64 ;\n\
         {MY_TIMES}: main ( -- )\n\
         0 0 Acc 0 5 [ | i | | acc |\n\
           acc Acc>\n\
           | x0 y0 |\n\
           x0 i +\n\
           y0 x0 i + +\n\
           Acc\n\
         ] my-times\n\
         Acc> . . ;\n"
    );
    let (stdout, code) = run_src("my-times-aggregate", &src);
    // `Acc>` destructures x then y (x deepest, y on top); `. .` prints top
    // first, so `y` (20) prints before `x` (10).
    assert_eq!(stdout, "20\n10\n");
    assert_eq!(code, 0);
}

// -- R18: nesting parity --------------------------------------------------

#[test]
fn my_times_nested_in_itself_produces_correct_output() {
    // R18: the outer loop (3 iterations) sums an inner `my-times` count (2
    // iterations, +1 each) into its own row each time: inner = 0+1 = 2 per
    // outer round, outer = 2+2+2 = 6. The inner call's `stack[..base]` picks
    // up the outer's own row underneath it, so the row mechanism composes
    // under nesting without extra plumbing.
    let src = format!(
        "{MY_TIMES}: main ( -- )\n\
         0 0 3 [ | i |\n\
           0 0 2 [ | j | 1 + ] my-times +\n\
         ] my-times . ;\n"
    );
    let (stdout, code) = run_src("my-times-nested", &src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
}

// -- R19: no regression ------------------------------------------------------

#[test]
fn combinators_library_contains_no_tilde() {
    // R19: `lib/combinators.sth` contains no `~`, i.e. 10a changed no
    // shipped signature to the new inline-quotation syntax; the intrinsic's
    // callers were not touched. (Byte-identity against history is already
    // covered, without a hardcoded SHA, by `tests/qbe_baseline.rs`'s
    // `corpus_qbe_stays_byte_identical_to_baseline`.)
    let current_src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lib/combinators.sth"),
    )
    .expect("reading lib/combinators.sth should succeed");
    assert!(
        !current_src.contains('~'),
        "lib/combinators.sth must contain no `~`"
    );
}

#[test]
fn while_is_unaffected_by_the_row_and_back_edge_rewrite() {
    // R19 (`while` half; the corpus/QBE-baseline half is
    // `tests/qbe_baseline.rs`, byte-identical against the same base commit).
    // `while`'s own back-edge shape (1<->1, no row) is the shape the R11
    // rewrite had to keep agreeing with; this is the value-level twin of
    // `while_self_tail_still_checks_after_back_edge_rewrite`
    // (`src/check.rs`), run end to end.
    let src = ": while ( 'a [ 'a -- 'a bool ] -- 'a )\n\
               | p | p call if p while else end ;\n\
               : main ( -- ) 0 [ dup 5 < if 1 + true else false end ] while . ;\n";
    let (stdout, code) = run_src("while-unaffected", src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
}
