//! P7.S3t goldens, phase 1: the call-site explicit type instantiation
//! `f[Point]` as a *surface syntax*. Phase 1 delivers the parse and the
//! rejections; nothing here observes an instantiation grounding a type
//! variable, because `check_poly_call` still ignores the list (phase 2 seeds
//! `Subst` from it).
//!
//! The rejections are the load-bearing half. Exactly one of `check_term`'s
//! dozen dispatch routes for a `Call` can consume a type-argument list -- the
//! polymorphic-call interception -- and a route that silently dropped one would
//! link whichever specialization it resolved on its own, which is a miscompile
//! rather than a diagnostic. So each of the routes below is pinned rejecting,
//! and the grammar this syntax narrows (a word followed by a *spaced* bracket)
//! is pinned still parsing as the quotation literal it always was.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

/// A scratch single-file program, removed with its directory on drop.
struct Scratch(PathBuf);

impl Scratch {
    /// Written verbatim: every assertion below pins a `line N` or `col C`, and
    /// several fixtures are *about* what a bracket adjacency means, so nothing
    /// is appended or rewritten by the harness.
    fn write(tag: &str, src: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3t-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.sth");
        std::fs::write(&path, src).unwrap();
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

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .arg("--manifest")
        .arg(common::fixture_manifest())
        .output()
        .expect("sooth build should spawn")
}

fn build_and_run(tag: &str, src: &str) -> String {
    let prog = Scratch::write(tag, src);
    let build = sooth_build(prog.path());
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(prog.path().with_extension(""))
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    String::from_utf8_lossy(&run.stdout).into_owned()
}

fn build_error(tag: &str, src: &str) -> String {
    let prog = Scratch::write(tag, src);
    let build = sooth_build(prog.path());
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

/// The message every non-polymorphic route reports for a list it cannot
/// consume. One string, so a test that names a route also pins that the route
/// reached *this* rejection rather than some unrelated failure.
fn takes_no_type_arguments(name: &str, line: u32) -> String {
    format!(
        "`{name}` (line {line}) takes no type arguments; \
         only a call to a polymorphic word may be explicitly instantiated"
    )
}

/// R2/R6: a glued list on a polymorphic call parses, survives resolution, and
/// the program runs. Phase 1 ignores the list downstream, so this witnesses the
/// syntax reaching `check_poly_call` at all -- not yet that it grounds
/// anything.
#[test]
fn an_explicit_instantiation_on_a_polymorphic_call_builds_and_runs() {
    let out = build_and_run(
        "poly-call",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : main ( -- ) 7 id[i64] . ;\n",
    );
    assert_eq!(out, "7\n");
}

/// R2's non-break witness, and the mutation subject for the adjacency
/// conjunct: with the column test deleted, both spaced brackets here become
/// explicit instantiations and this program stops building.
#[test]
fn a_spaced_bracket_after_a_word_is_still_a_quotation() {
    let out = build_and_run(
        "spaced-bracket",
        "import: intrinsics * ;\n\
         : main ( -- )\n\
           1 dup [ 2 add ] call add .\n\
           9 dup [ i64 ; 3 ] drop drop drop ;\n",
    );
    assert_eq!(out, "4\n");
}

/// R3, the builtin route: `dup` is dispatched by name ahead of the word
/// environment, so it never reaches the polymorphic interception and has
/// nowhere to put a list.
#[test]
fn a_glued_bracket_after_a_builtin_is_rejected() {
    let err = build_error(
        "glued-builtin",
        "import: intrinsics * ;\n\
         : main ( -- ) 1 dup[i64] . . ;\n",
    );
    assert!(err.contains(&takes_no_type_arguments("dup", 2)), "{err}");
}

/// R3, the local-read route: a body local wins over every word, so a list on
/// one names no callee at all.
#[test]
fn a_glued_bracket_after_a_local_is_rejected() {
    let err = build_error(
        "glued-local",
        "import: intrinsics * ;\n\
         : main ( -- ) 1 | x | x[i64] . ;\n",
    );
    assert!(err.contains(&takes_no_type_arguments("x", 2)), "{err}");
}

/// R3, the concrete `env` route: a monomorphic word has no type variables to
/// instantiate, so the list is a statement about a signature that does not
/// exist.
#[test]
fn a_glued_bracket_after_a_concrete_word_is_rejected() {
    let err = build_error(
        "glued-concrete",
        "import: intrinsics * ;\n\
         : inc ( i64 -- i64 ) 1 add ;\n\
         : main ( -- ) 1 inc[i64] . ;\n",
    );
    assert!(err.contains(&takes_no_type_arguments("inc", 3)), "{err}");
}

/// R7: a type *variable* argument. `parse_type_expr` has no production for one,
/// so without its own message this reads as a missing type declaration rather
/// than as the unsupported forwarding it is.
#[test]
fn a_type_variable_argument_is_rejected() {
    let err = build_error(
        "ty-var-argument",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : g ( 'U -- 'U ) id['U] ;\n\
         : main ( -- ) 1 g . ;\n",
    );
    assert!(
        err.contains(
            "`'U` (line 3, col 21) is a type variable; \
             an explicit instantiation takes concrete types"
        ),
        "{err}"
    );
    assert!(
        err.contains(
            "note: forwarding a caller's type variable through an explicit \
             instantiation is not supported"
        ),
        "{err}"
    );
    assert!(!err.contains("unknown type"), "{err}");
}

/// R1/R3: a call inside a polymorphic word's own body is checked symbolically,
/// against no `Subst`, so there is nothing for a list to seed. Rejected rather
/// than dropped -- and with its own message, because the remedy is one frame up,
/// at the enclosing word's own call site.
#[test]
fn an_instantiation_inside_a_polymorphic_body_is_rejected() {
    let err = build_error(
        "inside-poly-body",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : g ( 'U -- 'U ) drop 7 id[i64] ;\n\
         : main ( -- ) 1 g . ;\n",
    );
    assert!(
        err.contains(
            "`id` in `g` (line 3) cannot be explicitly instantiated inside a \
             polymorphic word's own body"
        ),
        "{err}"
    );
    assert!(
        err.contains("note: instantiate the enclosing word at its own call site instead"),
        "{err}"
    );
}

/// R10: the REPL rejects the syntax outright. A session routes through
/// `lower_instantiation` and skips the module-level checks this slice's
/// correctness argument rests on, so the guard fails closed instead of printing
/// success and binding whichever specialization the session found. Both line
/// shapes are covered, since a definition's body is walked separately from a
/// bare expression's terms.
#[test]
fn explicit_instantiation_is_rejected_at_the_repl() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("repl should spawn");
    let mut stdin = child.stdin.take().expect("stdin should be piped");
    stdin
        .write_all(b"1 id[i64] .\n: w ( -- ) 1 id[i64] drop ;\n")
        .expect("writing stdin should succeed");
    drop(stdin);
    let out = child.wait_with_output().expect("repl should exit cleanly");
    let session = String::from_utf8(out.stdout).expect("stdout should be utf8");
    let hits: Vec<&str> = session
        .lines()
        .filter(|l| l.contains("explicit type instantiation is not available at the REPL"))
        .collect();
    assert_eq!(
        hits.len(),
        2,
        "both line shapes reject; session was:\n{session}"
    );
    assert!(hits[0].contains("(line 1, col 3)"), "{session}");
    assert!(hits[1].contains("(line 1, col 14)"), "{session}");
    assert!(
        session.contains(
            "note: `f[Point]` needs a whole-program impl registry a live session \
             does not assemble"
        ),
        "{session}"
    );
    assert!(!session.contains("defined w"), "{session}");
}
