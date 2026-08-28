//! P7.S11 phase 1 goldens (R1/R2/R2.1/R3/R6): an unbounded `inline`
//! combinator's standalone check can ground a generic type its own declared
//! output (or a nested slot -- a quotation-input effect, an array element)
//! applies, without leaking a scratch-minted monomorph or shape into the live
//! registries. A top-level generic *input* slot stays rejected (R3), and the
//! def-site gate is lifted independently of the frozen call-site env, which
//! is out of scope here (see the brief).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s11-{}-{tag}-{seq}", std::process::id()));
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
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

fn build_error(src: &Path) -> String {
    driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .expect_err("this fixture must be rejected")
}

/// Golden 2: the R1-vs-R4 discriminator, deliberately constructor-free in its
/// own body (only `call`) -- `Ok`'s constructor resolves through the parse-
/// time monomorph `mki` grounds, not through R4's word-scoped env, so this
/// dies under "revert R1" and survives "stub R4".
#[test]
fn constructor_free_combinator_grounds_its_generic_output_and_runs() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : mki ( i64 -- Result[i64 i64] ) Ok ;\n\
         : relay inline ( 'T ~[ 'T -- Result['T i64] ] -- Result['T i64] ) call ;\n\
         : main ( -- )\n\
           7 ~[ Ok ] relay\n\
           ~[ ( Ok ) Ok> . ]\n\
           ~[ ( Err ) Err> . ]\n\
           Result? ;\n";
    let prog = Scratch::write("golden2", src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n");
}

/// Golden 3: a top-level generic *input* slot on a combinator stays rejected
/// (R3) -- verbatim the S12 fixture
/// (`a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire`),
/// so it cannot pass for an unrelated reason.
#[test]
fn combinator_over_a_generic_input_slot_still_rejected() {
    let src = "type: Option['T] | None | Some 'T ;\n\
         type: Pt x i64 y i64 ;\n\
         : probe inline ( Option['T] ~[ -- i64 ] -- i64 )\n\
           | f |\n\
           ~[ ( Some ) drop f call ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki ~[ 5 ] probe . ;\n";
    let prog = Scratch::write("golden3", src);
    let err = build_error(prog.path());
    assert!(
        err.contains("names the generic type `Option['T]`")
            && err.contains("cannot yet be instantiated at a variable-bearing application"),
        "expected the standing variable-bearing restriction, got: {err}"
    );
}

/// Golden 4 (P0-B): a body construction that type-directs on its own
/// grounded output decl (`tag`, which indexes the enum registry) must report
/// a diagnostic, not panic -- the flush into a word-scoped extended `enums`
/// slice is what gives the body walk a decl to index at all.
#[test]
fn a_body_indexing_its_grounded_output_decl_reports_not_panics() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : wrapd inline ( 'T ~[ 'T -- Result['T i64] ] -- i64 ) call tag ;\n\
         : main ( -- ) 7 ~[ 1 add Ok ] wrapd . ;\n";
    let prog = Scratch::write("golden4", src);
    let err = build_error(prog.path());
    assert!(
        err.contains("`tag` requires an enum whose variants all carry no payload")
            && err.contains("Result[i64 i64]"),
        "expected a located diagnostic, not a panic, got: {err}"
    );
}

/// Golden 8 (P0-C, R6): a combinator whose declared output is an array of its
/// own grounded monomorph must build and not panic at lowering -- `hold` is
/// never called, so this is a compile-only witness of R6's word-scoped
/// `arrays` registry: without it, the live `arrays` registry would carry a
/// decl whose element is a scratch-only, never-flushed `EnumId`, and lowering
/// would panic indexing it. No sibling monomorph exists in this fixture, so
/// the mint really is scratch, not a dedup onto something already flushed.
#[test]
fn a_combinator_returning_an_array_of_its_grounded_monomorph_builds_and_does_not_panic() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : hold inline ( 'T ~[ 'T -- Result['T i64] ] -- array[Result['T i64] 4] )\n\
           call 4 fill ;\n\
         : main ( -- ) 0 . ;\n";
    let prog = Scratch::write("golden8", src);
    let (_, code) = build_and_run(prog.path());
    assert_eq!(
        code, 0,
        "hold is never called; the build must not panic at lowering"
    );
}
