//! Phase 4 slice 10c, P2: the quotation-effect row gate.
//!
//! `~[ ..i -- ..o ]` with `..i != ..o` now parses for a quotation-taking
//! (always-inlined) word's own declared parameter (P2's parser lift). This
//! file checks the checker half: a hand-written, fully row-polymorphic `myif`
//! actually checks, runs, and produces the right values when its branches
//! change the carried region's shape, its two branch literals are reconciled
//! against each other rather than a fixed point (there is none to check,
//! R-P2-4), and a genuine contradiction between them is caught at the
//! argument site, not the splice site.

use sooth::ir::{lower, Instr, IrFunc, Terminator};
use sooth::{check, lexer, parser};

/// A row-polymorphic `if`, hand-written over the primitive `if` (the library
/// `if` arrives in P3): `..i`/`..o` may differ, per P2's parser lift.
const MYIF: &str = ": myif inline ( ..i bool ~[ ..i -- ..o ] ~[ ..i -- ..o ] -- ..o )\n\
     | e | | t | | c | c ~[ t call ] ~[ e call ] if ;\n";

fn lowered(src: &str) -> Vec<IrFunc> {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    lower(&module).expect("lowering should succeed").funcs
}

fn func<'f>(funcs: &'f [IrFunc], name: &str) -> &'f IrFunc {
    funcs
        .iter()
        .find(|f| f.name.starts_with(name))
        .unwrap_or_else(|| panic!("`{name}` is lowered"))
}

fn self_calls(f: &IrFunc) -> usize {
    f.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::Call(_, sym, _) if *sym == f.name))
        .count()
}

fn back_edges(f: &IrFunc) -> usize {
    f.blocks
        .iter()
        .filter(|b| matches!(b.term, Terminator::Jmp(target) if target.0 <= b.id.0))
        .count()
}

fn opens_a_loop_header(f: &IrFunc) -> bool {
    matches!(f.blocks[0].term, Terminator::Jmp(_))
}

static NEXT_TEMP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn build_binary(name: &str, src: &str) -> std::path::PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}-{id}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    std::fs::remove_file(&path).ok();
    binary
}

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

fn run(src: &str) -> String {
    let binary = build_binary("slice10c-rowgate", src);
    let out = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(out.status.success(), "binary exited nonzero");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

// -- E-P2-2: a shape-changing quotation checks, runs, prints correctly ------

#[test]
fn shape_changing_myif_checks_and_runs_with_a_surviving_carried_value() {
    // `..i` grounds to two carried `i64`s (the caller's `99 5`); each branch
    // leaves *three* values (`..o`), so the declared rows genuinely differ.
    // The bottom value (`99`) is below everything either branch reads or
    // writes, so it survives the call untouched -- the "carried value
    // survives" half of E-P2-2.
    let src = format!(
        "{MYIF}: demo ( i64 i64 bool -- i64 i64 i64 )\n\
         ~[ dup 10 + ] ~[ dup 20 + ] myif ;\n\
         : main ( -- ) 99 5 true demo . . . 99 5 false demo . . . ;\n"
    );
    let out = run(&src);
    assert_eq!(
        out, "15\n5\n99\n25\n5\n99",
        "the untouched carried value (99) and the branch's computed values both print correctly"
    );
}

// -- E-P2-3: a contradicting branch is rejected at the argument site --------

#[test]
fn contradicting_branch_output_is_rejected_at_the_argument_site() {
    // The `true` branch leaves the carried value plus one computed value
    // (net +1), the `false` branch drops it (net -1): they share one
    // declared `..o`, so the second literal checked contradicts the first
    // and must be rejected right there, naming both shapes and locating the
    // second literal -- not the generic splice-site message a bare
    // stack-depth mismatch would produce once the callee body is actually
    // spliced.
    let src = format!(
        "{MYIF}: demo ( i64 bool -- i64 )\n\
         ~[ dup 10 + ] ~[ drop ] myif ;\n"
    );
    let err = check_error(&src);
    assert_eq!(
        err,
        "error: the quotations passed to `myif` leave different stack shapes: \
         an earlier one leaves `i64 i64`, this one leaves nothing in `demo` (line 4)"
    );
}

// -- E-P2-4: the back-edge interaction is sound with a shape-changing myif -

#[test]
fn spliced_self_tail_through_shape_changing_myif_lowers_to_a_back_edge() {
    let src = format!(
        "{MYIF}: sum-to ( i64 i64 -- i64 )\n\
         | n | | acc | n 0 = ~[ acc ] ~[ acc n + n 1 - sum-to ] myif ;\n\
         : main ( -- ) 0 10 sum-to . ;\n"
    );
    let funcs = lowered(&src);
    let sum = func(&funcs, "sum");
    assert_eq!(self_calls(sum), 0, "the recursion is a loop, not a call");
    assert_eq!(back_edges(sum), 1, "the loop needs exactly one back-edge");
    assert!(opens_a_loop_header(sum));
    assert!(
        !funcs.iter().any(|f| f.name.starts_with("myif")),
        "a combinator mints no `IrFunc`"
    );
}

#[test]
fn spliced_self_tail_through_shape_changing_myif_runs_one_million_iterations_in_constant_stack() {
    let src = format!(
        "{MYIF}: sum-to ( i64 i64 -- i64 )\n\
         | n | | acc | n 0 = ~[ acc ] ~[ acc n + n 1 - sum-to ] myif ;\n\
         : main ( -- ) 0 1000000 sum-to . ;\n"
    );
    let binary = build_binary("slice10c-rowgate-1m", &src);
    let (code, out) = run_at_stack_limit(&binary, 512);
    std::fs::remove_file(&binary).ok();
    assert_eq!((code, out.as_str()), (Some(0), "500000500000"));
}
