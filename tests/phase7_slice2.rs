//! Phase 7 Slice 2 exit goldens: static storage and global sets, source in ->
//! diagnostic out.
//!
//! The global-set analysis runs in `driver::assemble_module`, not in
//! `check::check`, so every case here goes through a real file build rather
//! than the in-process single-file checker.
//!
//! The two remaining goldens the spec lists -- an agreeing static program
//! building and running, and a static ref captured into an escaping closure
//! being *admitted* without an ICE -- both need scalar-static lowering, which
//! is Phase 4: today a program that gets past the checker dies in
//! `lower_reference_word`. They land with that phase, not this one.

fn build_error(name: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let err = sooth::driver::build(&path).expect_err("build should fail its check");
    std::fs::remove_file(&path).ok();
    err
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
