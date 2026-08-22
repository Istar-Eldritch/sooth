//! Phase 7 Slice 3f goldens: a ground `Type::Quotation` value crossing the
//! polymorphism boundary -- the argument boundary (R1/R2), the body boundary
//! (R3), and the two composing. Negatives land alongside whichever phase's
//! fix they exercise.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3d.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3f-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prog.sth");
        std::fs::write(&path, contents).unwrap();
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

fn check_err(src: &str) -> String {
    let tokens = sooth::lexer::lex(src).unwrap();
    let mut module = sooth::parser::parse(&tokens).unwrap();
    sooth::check::check(&mut module).expect_err("this program should be rejected")
}

fn build_and_run(src: &Path) -> (PathBuf, String, i32) {
    let binary = driver::build(src).expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        binary,
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

/// R1/R2 behavioural: a poly word declaring both a real type variable and a
/// ground `Type::Quotation` parameter, called from a concrete body with a
/// literal quotation argument, run at two distinct instantiations of the
/// variable so it is carried rigidly rather than coincidentally matching.
#[test]
fn argument_boundary_materializes_ground_quotation_param() {
    let src = "import: intrinsics * ;\n\
               : run_it ( 'T: Copy [ i64 -- i64 ] -- 'T ) drop ;\n\
               : main ( -- )\n\
                 7 [ 1 add ] run_it .\n\
                 true [ 1 add ] run_it .\n\
               ;\n";
    let prog = Scratch::write("argument-boundary-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "7\ntrue\n",
        "each instantiation of `'T` must carry the materialized quotation argument independently"
    );
}

/// R3 behavioural: a poly word declaring a ground `Type::Quotation` parameter
/// and `call`ing it inside its own body -- a real `(code, env)` value, so the
/// body honours the declared effect instead of splicing a literal. Run at two
/// instantiations of its unrelated `'T`, which the quotation never touches.
#[test]
fn body_boundary_calls_ground_quotation_param() {
    let src = "import: intrinsics * ;\n\
               : call_it ( 'T: Copy [ i64 -- i64 ] -- 'T i64 )\n\
                 1 swap call\n\
               ;\n\
               : main ( -- )\n\
                 9 [ 1 add ] call_it . .\n\
                 true [ 1 add ] call_it . .\n\
               ;\n";
    let prog = Scratch::write("body-boundary-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "2\n9\n2\ntrue\n",
        "the called quotation must run in each instantiation, beside the untouched `'T`"
    );
}

/// R1/R2 and R3 composing: one poly word that both receives a ground
/// `Type::Quotation` argument across the call boundary and `call`s it in its
/// own body. The declared effect takes two inputs, so the body boundary pops
/// more than the single slot the goldens above exercise.
#[test]
fn argument_and_body_boundary_together() {
    let src = "import: intrinsics * ;\n\
               : apply_it ( 'T: Copy [ i64 i64 -- i64 ] i64 -- 'T i64 )\n\
                 3 rot call\n\
               ;\n\
               : main ( -- )\n\
                 9 [ add ] 4 apply_it . .\n\
                 true [ add ] 4 apply_it . .\n\
               ;\n";
    let prog = Scratch::write("round-trip-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "7\n9\n7\ntrue\n",
        "a two-input declared effect must pop both operands at the body boundary"
    );
}

/// L1 at the golden level: a declared quotation parameter that still carries a
/// free variable inside its brackets is out of scope for this slice, and
/// `call`ing it in a poly body keeps its pre-existing wording.
#[test]
fn body_boundary_rejects_an_abstract_quotation_param() {
    let err = check_err(
        "import: intrinsics * ;\n\
         : call_it ( 'T: Copy [ i64 -- 'T ] -- 'T )\n\
           1 swap call\n\
         ;\n\
         : main ( -- ) [ 5 ] call_it drop ;\n",
    );
    assert_eq!(
        err,
        "error: `call` is not permitted on a quotation in `call_it` (line 3)"
    );
}
