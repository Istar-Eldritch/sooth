//! P7.S12 phase 1 goldens (R8.1): the three live-at-`afd3d52` defects the
//! spec's B1/B2/B3 repros name -- a false rejection (`Eliminate`), a
//! miscompiled field read (`Destructure`), and a runtime death or build
//! failure (`Construct`) -- all three the same defect: a poly body's
//! generated-enum-word call resolves through a bare, last-write-wins key
//! shared by every monomorph of one header. Each repro runs in **both**
//! declaration orders: the defect *is* order sensitivity, so a single order
//! cannot witness a last-write-wins key.

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
            std::env::temp_dir().join(format!("sooth-p7s12-{}-{tag}-{seq}", std::process::id()));
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

/// Build and run `src`, asserting a clean (signal-free) exit -- a segfault or
/// backend panic is a failure here, not a missing line (R8's own rule).
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

const TYPES: &str = "type: Pair['A] | Nil | One 'A ;\n\
     type: Pt x i64 y i64 ;\n";

/// B1 (`Eliminate`): the `mk1`-then-`mk2` order -- false-rejected at
/// `afd3d52` (R1.1's fix). The registry's bare key must never decide which
/// monomorph a call site's own scrutinee names.
#[test]
fn eliminate_over_asymmetric_monomorphs_builds_mk1_then_mk2() {
    let src = format!(
        "{TYPES}\
         : mk1 ( i64 -- Pair[i64] ) One ;\n\
         : mk2 ( Pt -- Pair[Pt] ) One ;\n\
         : use ( Pair[i64] 'T -- i64 'T )\n\
           | keep |\n\
           ~[ ( One ) drop 1 ]\n\
           ~[ ( Nil ) drop 0 ]\n\
           Pair?\n\
           keep ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 5 use drop . ;\n"
    );
    let prog = Scratch::write("b1a", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n");
}

/// B1 (`Eliminate`): the `mk2`-then-`mk1` order, live-clean at `afd3d52`.
/// Must build and print identically to the order above.
#[test]
fn eliminate_over_asymmetric_monomorphs_builds_mk2_then_mk1() {
    let src = format!(
        "{TYPES}\
         : mk2 ( Pt -- Pair[Pt] ) One ;\n\
         : mk1 ( i64 -- Pair[i64] ) One ;\n\
         : use ( Pair[i64] 'T -- i64 'T )\n\
           | keep |\n\
           ~[ ( One ) drop 1 ]\n\
           ~[ ( Nil ) drop 0 ]\n\
           Pair?\n\
           keep ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 5 use drop . ;\n"
    );
    let prog = Scratch::write("b1b", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n");
}

/// B2 (`Destructure`): the `mk1`-then-`mk2` order -- false-rejected at
/// `afd3d52` for the same reason B1's identical order is.
#[test]
fn destructure_over_asymmetric_monomorphs_builds_mk1_then_mk2() {
    let src = format!(
        "{TYPES}\
         : mk1 ( i64 -- Pair[i64] ) One ;\n\
         : mk2 ( Pt -- Pair[Pt] ) One ;\n\
         : use ( Pair[i64] 'T -- i64 'T )\n\
           | keep |\n\
           ~[ ( One ) One> ]\n\
           ~[ ( Nil ) drop 0 ]\n\
           Pair?\n\
           keep ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 5 use drop . ;\n"
    );
    let prog = Scratch::write("b2a", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n");
}

/// B2 (`Destructure`): the `mk2`-then-`mk1` order -- live-clean at `afd3d52`,
/// but reads `Pair[Pt]`'s field layout out of a `Pair[i64]` value once R1.1
/// alone stops the false rejection above (R1.2/R1.3's fix). Must print the
/// same `7` as the order above.
#[test]
fn destructure_over_asymmetric_monomorphs_builds_mk2_then_mk1() {
    let src = format!(
        "{TYPES}\
         : mk2 ( Pt -- Pair[Pt] ) One ;\n\
         : mk1 ( i64 -- Pair[i64] ) One ;\n\
         : use ( Pair[i64] 'T -- i64 'T )\n\
           | keep |\n\
           ~[ ( One ) One> ]\n\
           ~[ ( Nil ) drop 0 ]\n\
           Pair?\n\
           keep ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 5 use drop . ;\n"
    );
    let prog = Scratch::write("b2b", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n");
}

/// B3 (`Construct`): the `mk2`-then-`mk1` order -- builds clean and then
/// segfaults at `afd3d52` (R1.2/R1.2a/R1.3's fix, since `wrap`'s `One` call
/// needs its per-θ instantiation grounded through the live instantiator). The
/// order that segfaults vs. the order that fails to build is a QBE
/// implementation detail of the last-write-wins collision, not a claim this
/// test depends on -- both orders are asserted clean either way.
#[test]
fn construct_inside_a_generic_word_builds_mk2_then_mk1() {
    let src = format!(
        "{TYPES}\
         : wrap ( 'T -- Pair['T] ) One ;\n\
         : mk2 ( Pt -- Pair[Pt] ) wrap ;\n\
         : mk1 ( i64 -- Pair[i64] ) wrap ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 drop ;\n"
    );
    let prog = Scratch::write("b3a", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "");
}

/// B3 (`Construct`): the `mk1`-then-`mk2` order -- does not even build at
/// `afd3d52` ("an aggregate field is copied by blit, not scalar-stored").
/// Must build and run clean, identically to the order above.
#[test]
fn construct_inside_a_generic_word_builds_mk1_then_mk2() {
    let src = format!(
        "{TYPES}\
         : wrap ( 'T -- Pair['T] ) One ;\n\
         : mk1 ( i64 -- Pair[i64] ) wrap ;\n\
         : mk2 ( Pt -- Pair[Pt] ) wrap ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 drop ;\n"
    );
    let prog = Scratch::write("b3b", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "");
}

/// R1.5: a combinator (`declares_inline`) whose own body constructs a
/// generic enum still ungrounded at its own type variable. A `Span`-keyed
/// `enum_words` cannot hold two splices' distinct resolutions for the one
/// body span this call sits at (R1.5's deliberate non-goal, no `(uid, span)`
/// widening), so this is a located rejection rather than a silent
/// last-write-wins collision behind a different door.
#[test]
fn combinator_constructing_ungrounded_generic_enum_is_rejected() {
    let src = "import: intrinsics * ;\n\
         type: Pair['A] | Nil | One 'A ;\n\
         trait: Foo['T] : bar inline ( 'T -- 'T ) ; ;\n\
         impl: Foo for i64\n\
           : bar | x | x ;\n\
         ;\n\
         : wrap_it inline ['T: Foo] ( 'T -- Pair['T] ) bar One ;\n\
         : main ( -- ) 1 wrap_it drop ;\n";
    let prog = Scratch::write("r15", src);
    let err =
        driver::build_with_manifest(prog.path(), common::manifest_for(prog.path()).as_deref())
            .expect_err("a combinator constructing an ungrounded generic enum is rejected");
    assert!(
        err.contains("constructs `Pair`")
            && err.contains("this combinator's own splice determines"),
        "expected a located R1.5 rejection naming the enum and the restriction, got: {err}"
    );
}
