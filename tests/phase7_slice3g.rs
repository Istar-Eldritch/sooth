//! Phase 7 Slice 3g goldens: self-recursion in a non-inline generic body.
//! A `Term::Call` naming the very poly word being walked typechecks against
//! that word's own signature (R1) and lowers to an ordinary recursive call
//! into whichever instantiation is being emitted (R2). A structural operand
//! mismatch at the self-call site is a located type error, not a check-time
//! loop or a panic (D1).
//!
//! R3's control -- a call to a *different* polymorphic word rejects with
//! `poly_calls_poly_word_error` -- retired with that diagnostic in P7.S3k,
//! which grounds such a call instead (`tests/phase7_slice3k.rs`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3d.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3g-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prog.sth");
        std::fs::write(&path, common::fixture_source("prog.sth", contents)).unwrap();
        Scratch(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

fn build_and_run(src: &Path) -> (String, i32) {
    let binary = driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

/// Behavioural: a **non-inline** generic word recursing into itself, down to a
/// base case, compiled and run. Called at two instantiations (`'T = i64` and
/// `'T = bool`) so `'T` is carried rigidly through the recursion rather than
/// coincidentally matching the only instantiation there is.
///
/// The recursive arm prints the counter before it recurses, so the *number of
/// recursion levels* is observable in stdout. That is what this golden
/// witnesses -- reaching the base case, the right number of times -- and not
/// *which* instantiation ran: the two bodies differ only in the type of the
/// `'T` they carry past the recursion, and a poly body cannot print its own
/// `'T` (`.` has no generic overload), so pinning both instantiations to one
/// symbol leaves this transcript unchanged. Callee identity is asserted
/// structurally instead, in `src/ir/driver.rs`: this body's self-call is in
/// tail position, so `poly_self_tail_call_lowers_to_loop_back_edge` is the
/// test that pins it -- each instantiation back-edging to its own header,
/// with no `Instr::Call` reaching any other symbol.
///
/// Deleting R2's lowering arm makes this die at lowering, on the poly
/// self-name `env` has no entry for. R1's checker arm is *not* pinned by this
/// transcript: with it stubbed, the S3k cross-call arm grounds the self-call
/// and stdout is unchanged. What breaks then is the loop shape -- the level is
/// reached as a cross-call, so it back-edges nowhere and recurses one frame
/// deep per level -- which the large-counter golden below and
/// `poly_self_tail_call_lowers_to_loop_back_edge` are the tests that catch.
#[test]
fn self_recursive_poly_word_runs_to_base_case() {
    let scratch = Scratch::write(
        "loopg",
        ": iszero ( i64 -- Bool ) 0 eq ;\n\
         : loopg ( 'T: Copy i64 -- 'T )\n\
           dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;\n\
         : main ( -- )\n\
           5 3 loopg .\n\
           True 2 loopg .\n\
         ;\n",
    );
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "3\n2\n1\n5\n2\n1\nTrue\n");
    assert_eq!(code, 0);
}

/// P7.S3g-follow's own exit criterion (roadmap: "a generic countdown over a
/// large counter runs in constant stack"), extending the base-case golden
/// above rather than migrating it: the *same* self-tail-recursive `loopg`,
/// this time counted down from far enough (one million) that one
/// `Instr::Call` per recursion level would overflow a reduced stack long
/// before reaching the base case, run under `ulimit -s 1024`. The loop body
/// prints nothing per iteration (a million-line transcript would swamp the
/// assertion, not strengthen it); only the final `'T` value is observable,
/// exactly as `poly_self_tail_call_lowers_to_loop_back_edge`
/// (`src/ir/driver.rs`) asserts the loop shape this depends on.
#[test]
fn self_recursive_poly_word_runs_a_large_counter_in_constant_stack() {
    let scratch = Scratch::write(
        "loopg-1m",
        ": iszero ( i64 -- Bool ) 0 eq ;\n\
         : loopg ( 'T: Copy i64 -- 'T )\n\
           dup iszero ~[ drop ] ~[ 1 sub loopg ] if ;\n\
         : main ( -- ) 7 1000000 loopg . ;\n",
    );
    let binary = driver::build_with_manifest(
        scratch.path(),
        common::manifest_for(scratch.path()).as_deref(),
    )
    .expect("program should build");
    let (code, stdout) = common::run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        (code, stdout.as_str()),
        (Some(0), "7"),
        "a self-tail poly word must run a million-deep countdown to completion in constant stack"
    );
}

fn check_err(src: &str) -> String {
    let tokens = sooth::lexer::lex(src).unwrap();
    let mut module = sooth::parser::parse(&tokens).unwrap();
    sooth::check::check(&mut module).expect_err("this program should be rejected")
}

/// Negative: a self-call whose operand window does not structurally match
/// the walking word's own `sig.inputs` is a located `type mismatch`, naming
/// the enclosing word, the self-call and the expected/found operand types --
/// never an infinite check-time loop and never a backend panic. This is D1's
/// actual termination witness: `'T` recursing at `Box['T]` is the shape the
/// roadmap's polymorphic-recursion hazard is about (a self-call at a
/// *different* type argument, which under monomorphizing codegen would
/// demand a fresh instantiation per level). An ordinary concrete-vs-concrete
/// mismatch (e.g. `bool` vs `i64`) exercises the same code path but is not
/// evidence for D1 specifically, since it says nothing about recursion at a
/// different type argument.
#[test]
fn self_call_recursing_at_a_different_type_argument_is_located_type_error() {
    let err = check_err(
        "type: Box 'T | Box 'T ;\n\
         : rec ( 'T i64 -- 'T )\n\
           drop Box 3 rec\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert_eq!(
        err,
        "error: type mismatch in `rec` (line 3)\n  `rec` expected `'T`, found `Box['T]`\n  note: declared ( -- )",
        "{err}"
    );
}

/// Negative: an ordinary concrete-vs-concrete operand mismatch ahead of a
/// self-call is likewise a located `type mismatch`, not a bespoke self-call
/// diagnostic -- the self-call arm reuses the same per-slot comparison any
/// other operand/signature mismatch goes through.
#[test]
fn self_call_concrete_operand_mismatch_is_located_type_error() {
    let err = check_err(
        "type: Bool | False | True ;\n\
         : rec ( 'T i64 -- 'T )\n\
           drop True rec\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert_eq!(
        err,
        "error: type mismatch in `rec` (line 3)\n  `rec` expected `i64`, found `Bool`\n  note: declared ( -- )",
        "{err}"
    );
}

/// Drive a REPL session over `input` in a fresh process, returning its stdout.
fn repl_session(input: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the sooth binary spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("the session exits");
    String::from_utf8(out.stdout).expect("stdout should be utf8")
}

/// P7 slice 3g-follow: the REPL's own instantiation lowering must reach the
/// same self-tail verdict the native path does. It is the one call site with
/// no IR to assert against (`emit_instantiations` hands its funcs straight to
/// the session's `.so`), so the witness is behavioural: a counter deep enough
/// that one real frame per level overflows the process stack, where a
/// back-edge runs it in constant space. Hardcoding `self_tail: false` here
/// aborts the session instead of printing.
///
/// The library imports are quoted-path, absolute: a REPL session can resolve
/// neither a module-name import nor a wildcard one, and `eq`/`if` are library
/// words, not intrinsics.
#[test]
fn repl_self_tail_poly_word_runs_a_deep_counter_in_constant_stack() {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("lib");
    let session = format!(
        "import: \"{cmp}\" c | eq | ;\n\
         import: \"{b}\" b | if | ;\n\
         : iszero ( i64 -- Bool ) 0 eq ;\n\
         : loopg ( 'T: Copy i64 -- 'T ) dup iszero ~[ drop ] ~[ 1 sub loopg ] if ;\n\
         7 2000000 loopg .\n",
        cmp = lib.join("cmp.sth").display(),
        b = lib.join("bool.sth").display(),
    );
    assert_eq!(
        repl_session(&session),
        "imported c\nimported b\ndefined iszero\ndefined loopg\n7\nstack: (empty)\n"
    );
}
