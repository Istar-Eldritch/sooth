//! Phase 7 Slice 3g goldens: self-recursion in a non-inline generic body.
//! A `Term::Call` naming the very poly word being walked typechecks against
//! that word's own signature (R1) and lowers to an ordinary recursive call
//! into whichever instantiation is being emitted (R2). A structural operand
//! mismatch at the self-call site is a located type error, not a check-time
//! loop or a panic (D1); a call to a *different* polymorphic word still
//! rejects with `poly_calls_poly_word_error` unchanged (R3).

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
/// recursion levels* is observable in stdout: a self-call lowered to the wrong
/// callee, or to a loop that ran the wrong number of times, changes the
/// transcript rather than merely the build. Deleting R1's checker arm makes
/// this fail to compile with `poly_calls_poly_word_error`; deleting R2's
/// lowering arm makes it die at lowering, on the poly self-name `env` has no
/// entry for.
#[test]
fn self_recursive_poly_word_runs_to_base_case() {
    let scratch = Scratch::write(
        "loopg",
        ": iszero ( i64 -- bool ) 0 eq ;\n\
         : loopg ( 'T: Copy i64 -- 'T )\n\
           dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;\n\
         : main ( -- )\n\
           5 3 loopg .\n\
           true 2 loopg .\n\
         ;\n",
    );
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "3\n2\n1\n5\n2\n1\ntrue\n");
    assert_eq!(code, 0);
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
        ": rec ( 'T i64 -- 'T )\n\
           drop true rec\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert_eq!(
        err,
        "error: type mismatch in `rec` (line 2)\n  `rec` expected `i64`, found `bool`\n  note: declared ( -- )",
        "{err}"
    );
}

/// Regression: a non-inline generic word calling a *second, different*
/// generic word still produces `poly_calls_poly_word_error` with its current
/// wording -- P7.S3k's gap, which this slice must not perturb (R3). Asserted
/// as a stable substring, not the full located string, since the diagnostic
/// interpolates a `(line, col)` that would otherwise bake a brittle position
/// into the golden.
#[test]
fn different_poly_word_call_still_names_the_narrowing() {
    let err = check_err(
        ": other ( 'T -- 'T ) ;\n\
         : caller ( 'T -- 'T ) other ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("cannot call the polymorphic word"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains(
            "a polymorphic word is not yet reachable from another polymorphic word across a module boundary"
        ),
        "unexpected message: {err}"
    );
}
