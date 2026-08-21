//! Phase 7 Slice 2 exit goldens: static storage and global sets, source in ->
//! diagnostic (or built binary) out.
//!
//! The global-set analysis runs in `driver::assemble_module`, not in
//! `check::check`, so every case here goes through a real file build rather
//! than the in-process single-file checker.
//!
//! The escaping-closure goldens assert an *admitted* program, not a
//! diagnostic: a static-rooted `owned_root` classifies as `OuterRooted`, so
//! `check::captures` never flags it, and unlike a local-rooted reference it
//! never can dangle -- a static outlives every closure that captures it. Its
//! job is to prove the admitted program lowers rather than ICEing, which is
//! the risk this codebase's materialized-quotation paths actually carry.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let seq = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sooth-p7s2-{}-{tag}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating the scratch dir should succeed");
    dir
}

fn build_error(name: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let err = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail its check");
    std::fs::remove_file(&path).ok();
    err
}

/// Build a scratch entry file and run it, returning `(stdout, exit code)`.
fn build_and_run(entry: &Path) -> (String, i32) {
    let binary = sooth::driver::build_with_manifest(entry, common::manifest_for(entry).as_deref())
        .expect("build should succeed");
    let out = std::process::Command::new(&binary)
        .output()
        .expect("the built binary should run");
    let dir = entry.parent().expect("the entry sits in a scratch dir");
    std::fs::remove_dir_all(dir).ok();
    (
        String::from_utf8(out.stdout).expect("stdout should be utf8"),
        out.status.code().expect("the process exits normally"),
    )
}

/// Write one entry file into a fresh scratch dir and build-and-run it.
fn run_program(tag: &str, src: &str) -> (String, i32) {
    let dir = scratch(tag);
    let entry = dir.join("main.sth");
    common::write_fixture(&entry, src).expect("writing the entry should succeed");
    build_and_run(&entry)
}

/// Exit case 1: an exported word whose declared set disagrees with what it
/// *transitively* touches. `tick` names no static itself; the mode it must
/// declare comes from `bump` through the intra-module call graph (R5).
#[test]
fn exported_word_global_set_mismatch_diagnostic() {
    let err = build_error(
        "p7s2-mismatch",
        "static: COUNT i64 = 0 ;\n\
         : bump ( -- ) &!COUNT 1 +! ;\n\
         : tick ( -- ) global: COUNT r bump ;\n\
         export: tick ;\n\
         : main ( -- ) tick ;\n",
    );
    assert!(
        err.contains(
            "`global:` entry `COUNT` of word `tick` (line 3, col 23) declares mode `r`, but the body infers `w`"
        ),
        "unexpected message: {err}"
    );
}

/// Exit case 2: an exported word touching a static with no `global:` clause at
/// all. The error names the static and the mode, and hands back the clause to
/// write (R6).
#[test]
fn undeclared_static_access_diagnostic() {
    let err = build_error(
        "p7s2-undeclared",
        "static: COUNT i64 = 0 ;\n\
         : tick ( -- ) &!COUNT 1 +! ;\n\
         export: tick ;\n\
         : main ( -- ) tick ;\n",
    );
    assert!(
        err.contains(
            "exported word `tick` (line 2, col 3) must declare its global set: it touches `COUNT` (w)"
        ) && err.contains("write `global: COUNT w` after the effect"),
        "unexpected message: {err}"
    );
}

/// Exit: a static-rooted reference reuses the type-keyed store rule unchanged
/// (R3). Nothing about the static branch routes around it.
#[test]
fn static_ref_escape_diagnostic() {
    let err = build_error(
        "p7s2-escape",
        "static: COUNT i64 = 0 ;\n: main ( -- ) &!COUNT ^ drop ;\n",
    );
    assert!(
        err.contains("a reference cannot be stored in `main`") && err.contains("`&!i64`"),
        "unexpected message: {err}"
    );
}

#[test]
fn duplicate_static_declaration_diagnostic() {
    let err = build_error(
        "p7s2-duplicate",
        "static: COUNT i64 = 0 ;\n\
         static: COUNT i64 = 1 ;\n\
         : main ( -- ) &!COUNT 1 +! ;\n",
    );
    assert!(
        err.contains("duplicate static `COUNT` (line 2, col 9); first declared at line 1, col 9"),
        "unexpected message: {err}"
    );
}

/// A static shares one name category with its module's words and types, so a
/// name already taken by either is a located error rather than a silent
/// shadowing at the borrow site.
#[test]
fn static_name_collides_with_word_or_type_diagnostic() {
    let err = build_error(
        "p7s2-collide-word",
        "static: COUNT i64 = 0 ;\n\
         : COUNT ( -- ) ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("static `COUNT` (line 1, col 9) is already the name of a word in this module"),
        "word collision: {err}"
    );

    let err = build_error(
        "p7s2-collide-type",
        "type: Count x i64 ;\n\
         static: Count i64 = 0 ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("static `Count` (line 2, col 9) is already the name of a type in this module"),
        "type collision: {err}"
    );
}

/// Exit (Phase 4): an agreeing program -- a private static counter, an
/// exported word declaring the global set it actually touches, incrementing it
/// through `&!` -- builds and runs. The counter starts at its declared
/// initialiser, so the printed value also pins that the initialiser reached
/// the emitted storage rather than being dropped for a zero slot.
#[test]
fn agreeing_static_program_builds_and_runs() {
    let (stdout, code) = run_program(
        "agreeing",
        "static: COUNT i64 = 40 ;\n\
         : tick ( -- ) global: COUNT w &!COUNT 1 +! ;\n\
         export: tick ;\n\
         : main ( -- ) tick tick &COUNT @ . ;\n",
    );
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

/// R2: a static is module-private and module-mangled, so two modules may each
/// declare `COUNT` and each gets its own storage. Both counters are read in
/// one program, so a lowering that collapsed them onto one data symbol shows
/// up as the wrong numbers, not a link error.
#[test]
fn two_modules_declaring_the_same_static_get_distinct_storage() {
    let dir = scratch("two-modules");
    common::write_fixture(
        &dir.join("lib.sth"),
        "static: COUNT i64 = 100 ;\n\
         : bump ( -- ) global: COUNT w &!COUNT 1 +! ;\n\
         : peek ( -- i64 ) global: COUNT r &COUNT @ ;\n\
         export: bump peek ;\n",
    )
    .unwrap();
    let entry = dir.join("main.sth");
    common::write_fixture(
        &entry,
        "import: \"lib.sth\" l | bump peek | ;\n\
         static: COUNT i64 = 7 ;\n\
         : main ( -- ) bump bump peek . &!COUNT 1 +! &COUNT @ . ;\n",
    )
    .unwrap();
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "102\n8\n");
    assert_eq!(code, 0);
}

/// R2, the exempt names: `resolve::mangle` deliberately leaves `main`, `drop`
/// and every `lib/core.sth` prelude word unmangled so a *word* of that name
/// stays reachable by bare name from a module that did not declare it. A
/// static is reachable no such way, so routing it through those exemptions
/// only ever collided: two modules each declaring `static: drop` emitted one
/// raw `drop` data symbol (`symbol `drop' is already defined` straight from
/// the assembler), and `Ctx::static_type`, which matches on the mangled name
/// alone, borrowed whichever of the two it found first. Values differ per
/// module and per name so a collapse that somehow linked would still show up
/// as wrong numbers. Both exempt classes that can name a static are covered:
/// the fixed names (`drop`) and the `lib/core.sth` prelude words (`if`).
#[test]
fn statics_named_like_mangle_exempt_words_get_distinct_storage() {
    let dir = scratch("exempt-names");
    for (file, dropv, ifv, word) in [
        ("lib1.sth", "100", "300", "peek1"),
        ("lib2.sth", "11", "13", "peek2"),
    ] {
        common::write_fixture(
            &dir.join(file),
            format!(
                "static: drop i64 = {dropv} ;\n\
                 static: if i64 = {ifv} ;\n\
                 : {word} ( -- i64 i64 ) global: drop r, if r &drop @ &if @ ;\n\
                 export: {word} ;\n"
            )
            .as_str(),
        )
        .unwrap();
    }
    let entry = dir.join("main.sth");
    common::write_fixture(
        &entry,
        "import: \"lib1.sth\" p | peek1 | ;\n\
         import: \"lib2.sth\" q | peek2 | ;\n\
         : main ( -- ) peek1 . . peek2 . . ;\n",
    )
    .unwrap();
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "300\n100\n13\n11\n");
    assert_eq!(code, 0);
}

/// R2, the `main` exemption specifically: a library static named `main` used
/// to reach the backend as the raw symbol `main`, which `qbe_name` rewrites to
/// `sooth_main` -- the very symbol the entry word owns and the C shim calls.
/// The assembler rejected the program outright, so this asserts a build that
/// links at all, with the static's value proving the two did not merge.
#[test]
fn a_library_static_named_main_does_not_collide_with_the_entry_symbol() {
    let dir = scratch("static-main");
    common::write_fixture(
        &dir.join("lib.sth"),
        "static: main i64 = 42 ;\n\
         : peekmain ( -- i64 ) global: main r &main @ ;\n\
         export: peekmain ;\n",
    )
    .unwrap();
    let entry = dir.join("main.sth");
    common::write_fixture(
        &entry,
        "import: \"lib.sth\" l | peekmain | ;\n\
         : main ( -- ) peekmain . ;\n",
    )
    .unwrap();
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

/// The spec's flagged high-risk case, half one: `&!COUNT` named *inside* a
/// quotation literal that materializes into a `(code, env)` value and escapes
/// its defining word. The borrow is taken when the closure runs, so the env is
/// empty -- the static's address is materialized inside the quotation body.
/// The job here is to prove **no ICE** -- this codebase has live
/// materialized-quotation crashes elsewhere, so "a static ref behaves like any
/// other ref" is exactly where it would break. It is admitted rather than
/// rejected because a static, unlike a local, outlives every closure that can
/// capture it; the calls through the escaped closure must reach the one shared
/// data symbol.
#[test]
fn static_ref_named_inside_an_escaping_quotation_no_ice() {
    let (stdout, code) = run_program(
        "escaping-closure",
        "static: COUNT i64 = 0 ;\n\
         : make ( -- [ -- ] ) global: COUNT w [ &!COUNT 1 +! ] ;\n\
         export: make ;\n\
         : main ( -- ) make | q | q call q call &COUNT @ . ;\n",
    );
    assert_eq!(stdout, "2\n");
    assert_eq!(code, 0);
}

/// Half two, the shape the test above does *not* reach: the `&!COUNT` is taken
/// in `make`'s own body and bound to a local, so the escaping closure carries
/// the live static-rooted reference in its captured `env` rather than
/// re-deriving the address per call. This is the case `check::captures`
/// classifies as `OuterRooted` and admits, and the one where a dangling-ref
/// analysis written for locals would have had to reject or miscompile.
#[test]
fn static_ref_captured_into_an_escaping_closure_env_no_ice() {
    let (stdout, code) = run_program(
        "escaping-closure-env",
        "static: COUNT i64 = 0 ;\n\
         : make ( -- [ -- ] ) global: COUNT w &!COUNT | c | [ c 1 +! ] ;\n\
         export: make ;\n\
         : main ( -- ) make | q | q call q call &COUNT @ . ;\n",
    );
    assert_eq!(stdout, "2\n");
    assert_eq!(code, 0);
}

/// D1/D3: every static type this slice accepts round-trips through emitted
/// storage, elided initialiser included -- `bool`'s zero is `false`, `str`'s is
/// the empty string, and a `u32` reads back at its own width rather than
/// through an oversized slot.
#[test]
fn every_scalar_static_type_round_trips_through_its_storage() {
    let (stdout, code) = run_program(
        "kinds",
        "static: FLAG bool = true ;\n\
         static: WIDE u32 ;\n\
         static: TAG str = \"hi\" ;\n\
         static: EMPTY str ;\n\
         : main ( -- )\n\
           &FLAG @ .\n\
           &!FLAG false !\n\
           &FLAG @ .\n\
           &WIDE @ .\n\
           &!WIDE 7 >u32 !\n\
           &WIDE @ .\n\
           &TAG @ .\n\
           &EMPTY @ .\n\
           0 . ;\n",
    );
    // `str` prints with no trailing newline, so `TAG` and the empty `EMPTY`
    // run into the final `0`.
    assert_eq!(stdout, "true\nfalse\n0\n7\nhi0\n");
    assert_eq!(code, 0);
}
