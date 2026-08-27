//! Phase 4 slice 10c, P1: shared tail-splice recognition.
//!
//! A self-recursive word whose recursive call sits inside a quotation handed to
//! a combinator that `call`s that parameter in tail position is a *self-tail*
//! word: the splice runs in place of the call, so the literal inherits the
//! caller's tail position and the recursion is a loop back-edge. Before this
//! slice nothing recognised the shape and the word blew the host stack.
//!
//! The combinators here are hand-written over the primitive `if` (the library
//! `if` arrives in P3), and their quotation effects carry no rows (a
//! shape-changing `~[ ..i -- ..o ]` is P2).

use sooth::ir::{lower, Instr, IrFunc, Terminator};
use sooth::{check, lexer, test_support};

mod common;

/// A two-way branch whose two quotation parameters are each `call`ed in tail
/// position, so both inherit their caller's tail position.
const BOOL_Q: &str = ": decide inline ( Bool ~[ -- i64 ] ~[ -- i64 ] -- i64 )\n\
     | e | | t | | c | c ~[ t call ] ~[ e call ] if ;\n";

/// Recon 4's negative twin: each arm `call`s one parameter and *then* drops the
/// other, so `drop` holds the tail position and neither parameter is
/// tail-called. Identical callers must stay ordinary recursion.
const BOOL_D: &str = ": decide! inline ( Bool ~[ -- i64 ] ~[ -- i64 ] -- i64 )\n\
     | e | | t | | c | c ~[ t call e drop ] ~[ e call t drop ] if ;\n";

fn sum_to(branch: &str, iterations: u32) -> String {
    format!(
        "{branch}: sum-to ( i64 i64 -- i64 )\n\
         | n | | acc | n 0 eq ~[ acc ] ~[ acc n add n 1 sub sum-to ] {caller} ;\n\
         : main ( -- ) 0 {iterations} sum-to . ;\n",
        caller = if branch == BOOL_Q {
            "decide"
        } else {
            "decide!"
        }
    )
}

fn lowered(src: &str) -> Vec<IrFunc> {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    lower(&module).expect("lowering should succeed").funcs
}

fn func<'f>(funcs: &'f [IrFunc], name: &str) -> &'f IrFunc {
    funcs
        .iter()
        .find(|f| f.name.starts_with(name))
        .unwrap_or_else(|| panic!("`{name}` is lowered"))
}

/// Calls the function makes to itself.
fn self_calls(f: &IrFunc) -> usize {
    f.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::Call(_, sym, _) if *sym == f.name))
        .count()
}

/// Jumps to *the loop's own header*, the target `begin_loop` seals block 0's
/// `Jmp` into (`opens_a_loop_header`, below, locates the fact a header was
/// opened at all; this locates which block it is and counts real back-edges
/// to it). Not "any block whose `Jmp` target id is <= its own": a spliced
/// eliminator's join block is also reached by an earlier-numbered arm's jump
/// (an ordinary forward-branch-then-merge shape, no loop involved), which the
/// old id-comparison heuristic miscounted as a back-edge the moment a
/// comparison's `Ordering?` splices inside a self-tail loop's body.
fn back_edges(f: &IrFunc) -> usize {
    let Terminator::Jmp(header) = f.blocks[0].term else {
        return 0;
    };
    f.blocks[1..]
        .iter()
        .filter(|b| matches!(b.term, Terminator::Jmp(target) if target == header))
        .count()
}

/// Whether a loop was opened at all: `begin_loop` seals the entry block with a
/// jump into the header it just made, where a loop-free body falls straight
/// into its first real terminator. Asserted separately from `back_edges`
/// because a rule that wrongly *recognises* a self-tail opens the header and
/// then never back-edges, which the back-edge count alone cannot see.
fn opens_a_loop_header(f: &IrFunc) -> bool {
    matches!(f.blocks[0].term, Terminator::Jmp(_))
}

static NEXT_TEMP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn build_binary(name: &str, src: &str) -> std::path::PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}-{id}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    std::fs::remove_file(&path).ok();
    binary
}

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

/// Run a scripted REPL session (one input line per element) and return the
/// captured stdout, mirroring `tests/phase1.rs`'s harness.
fn run_session(lines: &[&str]) -> String {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("repl")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("repl should spawn");
    let mut stdin = child.stdin.take().expect("stdin should be piped");
    let script = lines.join("\n") + "\n";
    std::io::Write::write_all(&mut stdin, script.as_bytes()).expect("writing stdin should succeed");
    drop(stdin);
    let output = child.wait_with_output().expect("repl should exit cleanly");
    assert!(
        output.status.success(),
        "repl exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

// -- E-P1-1: the spliced self-tail lowers to a back-edge ---------------------

#[test]
fn spliced_self_tail_lowers_to_a_back_edge() {
    let funcs = lowered(&sum_to(BOOL_Q, 10));
    let sum = func(&funcs, "sum");
    assert_eq!(
        self_calls(sum),
        0,
        "the recursion is a loop, not a call: {:?}",
        sum.blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .collect::<Vec<_>>()
    );
    assert_eq!(back_edges(sum), 1, "the loop needs exactly one back-edge");
    assert!(opens_a_loop_header(sum));
    assert!(
        !funcs.iter().any(|f| f.name.starts_with("Bool")),
        "a combinator mints no `IrFunc`: {:?}",
        funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
}

// -- E-P1-2: constant stack, and the right answer ---------------------------

#[test]
fn spliced_self_tail_runs_one_million_iterations_in_constant_stack() {
    // Both halves matter. A 512 KB stack overflows after a few thousand real
    // frames, so exit 0 pins the loop transform; the printed sum pins the
    // carried phis, which a back-edge wired to the wrong block would corrupt
    // while still exiting 0.
    let binary = build_binary("slice10c-splice-1m", &sum_to(BOOL_Q, 1_000_000));
    let (code, out) = common::run_at_stack_limit(&binary, 512);
    std::fs::remove_file(&binary).ok();
    assert_eq!((code, out.as_str()), (Some(0), "500000500000"));
}

// -- E-P1-3: discard-after-call stays ordinary recursion (recon 4) -----------

#[test]
fn discard_after_the_parameter_call_stays_ordinary_recursion() {
    // `t call e drop` puts `drop` in tail position, so the literal carrying the
    // self-call is not spliced at a tail and the word legitimately keeps its
    // real recursion. Asserted on the IR: a test that only checked "it builds"
    // would pass under a rule that blanket-accepted a trailing combinator call.
    let funcs = lowered(&sum_to(BOOL_D, 10));
    let sum = func(&funcs, "sum");
    assert_eq!(self_calls(sum), 1, "the self-call must survive as a call");
    assert_eq!(back_edges(sum), 0, "no loop is built for this shape");
    assert!(
        !opens_a_loop_header(sum),
        "no loop header either: a rule that blanket-accepted a trailing \
         combinator call would open one and leave it back-edgeless"
    );
}

// -- Review fix (Phase 1): a decline still checks as ordinary recursion -----

#[test]
fn forwarded_recursion_through_a_mid_body_bind_declines_the_loop_but_still_checks() {
    // R-P1-3: `rec`'s bind follows the quotation literal itself, a non-leading
    // term, so `param_binds` never tracks it and `TailWalk` cannot see through
    // the local to walk the literal it holds -- the walk declines,
    // `has_self_tail_call` is `False`, and lowering keeps ordinary recursion
    // for `spin`, never a loop.
    //
    // Before the review fix, the checker's back-edge-only reference guard
    // fired anyway: the positional `tail` flag threads through a resolved
    // value (not a name), so it sees through the bind and reaches `spin`'s
    // recursive call still marked tail, and the guard rejected `&!x` (a
    // reference into a fresh local `x` created inside `rec`'s own literal) as
    // crossing a loop back-edge that lowering was never going to build. Each
    // recursive call is a fresh call frame, not a shared loop iteration, so
    // the reference is safe; the two sides just disagreed about whether this
    // was a loop.
    let src = "type: V x i64 ;\n\
        : decide inline ( Bool ~[ -- ] ~[ -- ] -- )\n\
        | e | | t | | c | c ~[ t call ] ~[ e call ] if ;\n\
        : spin ( &!V i64 -- )\n\
        | r n |\n\
        ~[ 0 V | x | &!x n 1 sub spin ] | rec |\n\
        n 0 eq ~[ ] rec decide ;\n\
        : main ( -- )\n\
        0 V | v | &!v 3 spin ;\n";
    let funcs = lowered(src);
    let spin = func(&funcs, "spin");
    assert_eq!(
        self_calls(spin),
        1,
        "the decline keeps ordinary recursion, not a loop"
    );
    assert_eq!(back_edges(spin), 0, "no loop is built for this shape");
    assert!(!opens_a_loop_header(spin));
}

// -- E-P1-4: the REPL lowering path shares the predicate --------------------

#[ignore = "P7.S3s review finding: the REPL's `splice_import` binds a \
    concrete word (`w.poly.is_none()`) or retains a combinator \
    (`declares_inline`); it has no case for an imported non-inline poly \
    word, a category R5 creates for the first time (the six comparisons \
    all lost `inline`). Every comparison is unusable from the REPL now, \
    not just a session's own `'T: Copy Ord` declaration (R8's narrower, \
    already-accepted scope) -- fixing this needs the REPL to support \
    calling/monomorphizing a non-inline generic word, a separate slice."]
#[test]
fn repl_defined_spliced_self_tail_loops_in_constant_stack() {
    // R-P1-5 names `ir::lower_word` (the REPL's per-line entry) as one of the
    // sites that must consult the shared predicate. It takes its own
    // `CombinatorIndex` argument, so passing an empty one there would silently
    // revert this path to the pre-slice `If`-only walk while the whole-program
    // build path stayed correct -- exactly the divergence E-P1-4 exists to
    // catch, and the only one of the sites that lacked a witness which is
    // reachable at all (see the spec's E-P1-4 note).
    //
    // 1e6 real frames overflow the host stack, so the printed sum is what
    // separates the loop from the recursion; the REPL echoes the whole
    // residual stack each line, so the assertion pins the exact line.
    // P8.S2 (R3): the session imports the typed core rather than being seeded
    // with it.
    let cmp = common::repl_core_import("cmp", "eq");
    let boolean = common::repl_core_import("bool", "if");
    let out = run_session(&[
        &cmp,
        &boolean,
        ": decide inline ( Bool ~[ -- i64 ] ~[ -- i64 ] -- i64 ) \
         | e | | t | | c | c ~[ t call ] ~[ e call ] if ;",
        ": sum-to ( i64 i64 -- i64 ) \
         | n | | acc | n 0 eq ~[ acc ] ~[ acc n add n 1 sub sum-to ] decide ;",
        "0 1000000 sum-to",
    ]);
    assert!(
        out.lines().any(|l| l == "stack: 500000500000"),
        "the REPL-defined self-tail must loop and print its sum: {out}"
    );
}

// -- E-P1-5: the linear spine across the spliced back-edge -------------------

#[test]
fn linear_value_across_the_spliced_back_edge_is_error() {
    // The R15 guard reaches the spliced back-edge because the checker threads
    // the same tail flag lowering does. Without the extension the self-call is
    // not a tail call and the program is rejected for a different reason (`s`
    // unconsumed at the end of the body), so the exact message, the offending
    // type and the location are all part of the assertion.
    let src = format!(
        "type: Spy tag i64 ;\n\
         : drop ( Spy -- ) | s | \"drop \" . s Spy> . ;\n\
         {BOOL_Q}: spin ( i64 -- i64 )\n\
         | n | 9 Spy | s |\n\
         n 0 eq ~[ 0 ] ~[ n 1 sub spin ] decide ;\n\
         : main ( -- ) 3 spin . ;\n"
    );
    let err = check_error(&src);
    assert!(
        err.contains("linear values across a loop are not supported yet"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
    assert!(err.contains("line 7"), "the error should be located: {err}");
}

#[test]
fn linear_value_forwarded_into_the_spliced_back_edge_is_ok() {
    // Moved *into* the self-call's arguments the resource is forwarded, not
    // stranded, so the guard must not fire -- and the loop really is built
    // (the `Spy` rides the carried row) and disposes exactly once.
    let src = "type: Spy tag i64 ;\n\
        : drop ( Spy -- ) | s | \"drop \" . s Spy> . ;\n\
        : decide inline ( Spy Bool ~[ Spy -- i64 ] ~[ Spy -- i64 ] -- i64 )\n\
        | e | | t | | c | c ~[ t call ] ~[ e call ] if ;\n\
        : spin ( Spy i64 -- i64 )\n\
        | n | n 0 eq ~[ | s | s drop 0 ] ~[ | s | s n 1 sub spin ] decide ;\n\
        : main ( -- ) 0 Spy 3 spin . ;\n";
    let funcs = lowered(src);
    let spin = func(&funcs, "spin");
    assert_eq!(self_calls(spin), 0, "the forwarded case still loops");
    assert_eq!(back_edges(spin), 1);

    let binary = build_binary("slice10c-splice-linear", src);
    let (code, out) = common::run_at_stack_limit(&binary, 512);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        (code, out.as_str()),
        (Some(0), "drop 0\n0"),
        "the resource is disposed once, at the base case"
    );
}

// -- Phase 1 (test-harness soundness): back_edges repaired ------------------

/// `cmp` is already `inline` today (`lib/cmp.sth:39`), so a self-tail loop
/// whose body splices an `Ordering?` eliminator already produces the
/// false-positive join-block shape the old id-comparison heuristic
/// miscounted -- before the six comparisons themselves are flipped (P7.S8).
/// Built by hand over the raw `ult`/`ugt`/`branch` primitives (not `cmp`,
/// which needs a generic `'T: Ord` body to dispatch bare) so the witness
/// needs no flip: `Ordering?`'s eliminator allocates its join block before
/// its arm blocks, and the `Greater` arm's self-tail-spliced jump to that
/// join is a second, earlier-numbered-target `Jmp` the old helper counted as
/// a back-edge alongside the real one to the loop header.
#[test]
fn back_edges_repaired_helper_ignores_a_spliced_eliminators_join_block() {
    let src = "\
: countdown ( i64 -- i64 )\n\
  | n |\n\
  n 0 ult [ Less ] [ n 0 ugt [ Greater ] [ Equal ] branch ] branch\n\
  ~[ ( Less )    drop n ]\n\
  ~[ ( Equal )   drop n ]\n\
  ~[ ( Greater ) drop n 1 sub countdown ]\n\
  Ordering? ;\n\
: main ( -- ) 5 countdown . ;\n";
    let funcs = lowered(src);
    let countdown = func(&funcs, "countdown");
    assert_eq!(
        self_calls(countdown),
        0,
        "the recursion is a loop, not a call"
    );
    assert!(
        opens_a_loop_header(countdown),
        "the loop must actually open"
    );

    fn old_back_edges(f: &IrFunc) -> usize {
        f.blocks
            .iter()
            .filter(|b| matches!(b.term, Terminator::Jmp(target) if target.0 <= b.id.0))
            .count()
    }
    assert_eq!(
        old_back_edges(countdown),
        2,
        "the old any-backwards-jump heuristic double-counts the eliminator's join block"
    );
    assert_eq!(
        back_edges(countdown),
        1,
        "the repaired helper counts only the jump to the loop's own header"
    );
}

/// A single-variant enum eliminator's block 0 also ends in a `Jmp`
/// (`dispatch_on_tag`'s `n == 1` case, `func_builder/quotation.rs:355`,
/// unconditional dispatch to the lone variant block -- no comparison
/// needed), so `opens_a_loop_header` reads `true` here despite no loop ever
/// being built. `back_edges` must not ride on `opens_a_loop_header` (e.g. by
/// returning it as `0`/`1`): the true count is `0`, since nothing jumps back
/// to block 1, and this is the fixture that would catch a repair that
/// silently collapsed to that shortcut.
#[test]
fn back_edges_is_zero_for_a_single_variant_eliminators_unconditional_jump() {
    let src = "\
type: Uno\n\
| Solo\n\
;\n\
: pick ( -- Uno ) Solo ;\n\
: main ( -- ) pick ~[ ( Solo ) drop 1 . ] Uno? ;\n";
    let funcs = lowered(src);
    let main = func(&funcs, "main");
    assert!(
        opens_a_loop_header(main),
        "block 0 ends in an unconditional `Jmp`, same shape as a real loop header"
    );
    assert_eq!(
        back_edges(main),
        0,
        "no loop was built, so there is no back-edge to count"
    );
}
