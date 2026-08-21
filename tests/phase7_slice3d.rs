//! Phase 7 Slice 3d goldens: the two rowless quotation-consumer splices in a
//! **non-inline polymorphic** body. This file lands C1 (`call` on a
//! body-local literal, splicing its body in place) in phase 1; C2 (a literal
//! passed to a concrete `env` word with a ground `Type::Quotation` input)
//! and the shared negatives land in phase 2.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3b.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3d-{}-{tag}-{seq}", std::process::id()));
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

fn check_err(src: &str) -> String {
    let tokens = sooth::lexer::lex(src).unwrap();
    let mut module = sooth::parser::parse(&tokens).unwrap();
    sooth::check::check(&mut module).expect_err("this program should be rejected")
}

/// C1 behavioural: a non-inline generic word whose literal body names a
/// bound local, run at two distinct instantiations of `'T` so the splice is
/// carried rigidly rather than coincidentally matching.
#[test]
fn c1_call_on_literal_splices_body_in_place() {
    let src = ": bump ( 'T: Copy -- 'T 'T )\n\
               | x | [ x x ] call\n\
               ;\n\
               : main ( -- )\n\
                 5 bump . .\n\
                 true bump . .\n\
               ;\n";
    let prog = Scratch::write("c1-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "5\n5\ntrue\ntrue\n",
        "each instantiation must carry `x x` through the splice independently"
    );
}

/// C1 negative: `call` on a **non-literal** (declared/forwarded) quotation
/// operand in a non-inline poly body is a located rejection, not a panic
/// (L1). The parameter's own effect carries a free `'T`, so it stays
/// `PolyType::Quotation` rather than folding to `PolyType::Concrete`; R1's
/// new arm reuses `poly_op_on_variable_error`'s renderer, which names that
/// variant "a quotation" -- distinct from a `PolyType::QuotLit` top's "a
/// quotation literal".
#[test]
fn c1_call_on_non_literal_operand_is_located_rejection() {
    let err = check_err(
        ": caller ( 'T [ 'T -- 'T ] -- 'T i64 )\n\
           call\n\
         ;\n\
         : main ( -- ) 1 caller drop drop ;\n",
    );
    assert_eq!(
        err, "error: `call` is not permitted on a quotation in `caller` (line 2)",
        "{err}"
    );
    assert!(!err.contains("unknown word"), "{err}");
}
