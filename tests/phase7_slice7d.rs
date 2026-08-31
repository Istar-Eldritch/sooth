//! P7.S7d phase 1 (R14): a user `extern:` declaring an `f64` parameter
//! delivers that double to the C callee. The probe round's P8 control
//! (`slice7d-probes.md`) compiled and ran but printed `0`, because the backend
//! spelled extern calls fully fixed and QBE therefore emitted no `%al` setup:
//! a variadic C callee reads `%al` to decide whether to spill the xmm
//! registers its `va_arg` then reads back, so the double was never saved and
//! `snprintf` formatted whatever the register save area held.
//!
//! These are build-and-run goldens rather than IL assertions on purpose: a
//! wrong spelling compiles, links, and silently misprints, so only running the
//! binary settles it. The IL form itself is pinned beside `emit_instr`
//! (`src/backend/qbe.rs`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s7d-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build_and_run(t: &Tree, main: &str) -> String {
    t.write("sooth.pkg", "package: s7d ;\nlayer: hosted ;\n");
    let entry = t.write("main.sth", main);
    let binary = driver::build(&entry).expect("the fixture should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("the binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

/// The P8 control program's externs: `snprintf("%g\n", 2.5)` into a 64-byte
/// buffer, then `write(2)` of the formatted bytes.
const P8_EXTERNS: &str = "import: intrinsics * ;\n\
     extern: g-fmt ( &!array[u8 64] usize cstr f64 -- i32 ) \"snprintf\" ;\n\
     extern: sys-write ( i32 &!array[u8 64] usize -- isize ) \"write\" ;\n\
     extern: sys-strlen ( cstr -- usize ) \"strlen\" ;\n";

/// The spec's P8 control, verbatim. It passes *with or without* the fix: the
/// `fill` loop leaves its counter (64) in `%rax`, `snprintf` reads that as a
/// nonzero `%al` and spills the xmm registers anyway, and the double arrives
/// by luck. Kept as the spec's named exit artifact; the guard is the test
/// below.
#[test]
fn extern_f64_argument_reaches_variadic_callee() {
    let t = Tree::new("snprintf");
    let stdout = build_and_run(
        &t,
        &format!(
            "{P8_EXTERNS}\
             : main ( -- )\n\
             0 >u8 64 fill | buf |\n\
             &!buf 64 >usize \"%g\\n\" cstr 2.5 g-fmt >usize | n |\n\
             1 >i32 &!buf n sys-write drop\n\
             buf drop ;\n"
        ),
    );
    assert_eq!(stdout, "2.5\n", "the f64 argument reached snprintf");
}

/// The same program with one extra extern call ahead of it. `strlen("")`
/// returns 0, so `%al` is 0 on entry to `snprintf` unless the call itself sets
/// it -- which is exactly what the fixed spelling failed to do. Without the
/// preceding call the bug hides: `fill`'s loop counter leaves a nonzero `%rax`
/// behind, `snprintf` spills the xmm registers anyway, and the double arrives
/// by luck. The zero-%al premise is lowering-contingent (nothing guarantees
/// the two calls keep %rax clear between them); the deterministic half of the
/// guard is `emit_extern_call_with_f64_arg_is_all_args_variadic`, which pins
/// the spelling itself.
#[test]
fn extern_f64_argument_survives_a_zeroed_al_register() {
    let t = Tree::new("snprintf-zeroed-al");
    let stdout = build_and_run(
        &t,
        &format!(
            "{P8_EXTERNS}\
             : main ( -- )\n\
             0 >u8 64 fill | buf |\n\
             \"\" cstr sys-strlen drop\n\
             &!buf 64 >usize \"%g\\n\" cstr 2.5 g-fmt >usize | n |\n\
             1 >i32 &!buf n sys-write drop\n\
             buf drop ;\n"
        ),
    );
    assert_eq!(stdout, "2.5\n", "the f64 argument reached snprintf");
}

/// The spelling is applied to every extern, including non-variadic ones: a C
/// function that is not variadic ignores `%al`, so `strlen`/`write` are
/// unaffected. This is the guard on that claim.
#[test]
fn extern_calls_still_run_under_the_variadic_spelling() {
    let t = Tree::new("write-only");
    let stdout = build_and_run(
        &t,
        &format!(
            "{P8_EXTERNS}\
             : main ( -- )\n\
             0 >u8 64 fill | buf |\n\
             &!buf 64 >usize \"hi\\n\" cstr 0.0 g-fmt drop\n\
             \"hi\\n\" cstr sys-strlen | n |\n\
             1 >i32 &!buf n sys-write drop\n\
             buf drop ;\n"
        ),
    );
    assert_eq!(
        stdout, "hi\n",
        "strlen's count and write's bytes both survive"
    );
}

// -- phase 2 (R1-R13, R17): the retired intrinsic, printing through the library

/// Build and run against a package that names this checkout's own `lib/`, so a
/// fixture can `import: hosted::show` for real.
fn build_and_run_hosted(t: &Tree, main: &str) -> String {
    t.write(
        "sooth.pkg",
        &format!(
            "package: s7d ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write("main.sth", main);
    let binary = driver::build(&entry).expect("the fixture should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("the binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

fn build_error(t: &Tree, main: &str) -> String {
    t.write(
        "sooth.pkg",
        &format!(
            "package: s7d ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write("main.sth", main);
    driver::build(&entry).expect_err("the fixture should not build")
}

/// R6/R17, the whole printable vocabulary in one program, against the probe
/// round's pre-retirement `od` capture (`slice7d-probes.md`, "All-paths
/// baseline"). Byte-identical to the intrinsic's output with one deliberate
/// exception: `True`/`False` print lowercase now, since `core::bool`'s
/// capitalised overload is gone and `Show for Bool` spells them
/// `true`/`false` (R8/R4'). Note the two str rows carry no newline of their
/// own, while every numeric row appends one -- that per-type convention is
/// the byte compatibility R17 promises.
#[test]
fn every_printable_type_reproduces_the_pre_retirement_bytes() {
    let t = Tree::new("all-paths");
    let stdout = build_and_run_hosted(
        &t,
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: hosted::show | . | ;\n\
         : main ( -- )\n\
         42 .\n\
         -7 .\n\
         255 >u8 .\n\
         -5 >i8 .\n\
         100000 >u64 .\n\
         3.5 .\n\
         2.5 >f32 .\n\
         True .\n\
         False .\n\
         \"a\\tb\" .\n\
         \"hi\" cstr .\n\
         \"one\\ntwo\\n\" . ;\n",
    );
    assert_eq!(
        stdout, "42\n-7\n255\n-5\n100000\n3.5\n2.5\ntrue\nfalse\na\tbhione\ntwo\n",
        "the probe baseline, with `True`/`False` lowercased (R8)"
    );
}

/// R6': the widths the census found no corpus site for (`i16`/`u16`/`i32`/
/// `u32`) print too, and the signed ones widen before their sign test (the P7
/// gotcha: a bare `n 0 lt` on a narrow operand resolves the literal
/// ambiguously and errors, so a missing widening is a compile failure here,
/// not a wrong digit).
#[test]
fn the_rest_of_the_integer_tower_prints() {
    let t = Tree::new("width-tower");
    let stdout = build_and_run_hosted(
        &t,
        "import: intrinsics * ;\n\
         import: hosted::show | . | ;\n\
         : main ( -- )\n\
         -300 >i16 .\n\
         -70000 >i32 .\n\
         300 >u16 .\n\
         70000 >u32 .\n\
         -9 >isize .\n\
         9 >usize . ;\n",
    );
    assert_eq!(stdout, "-300\n-70000\n300\n70000\n-9\n9\n");
}

/// R4 (probe P3f): one selective import, then a *bare* `.` at every call site,
/// dispatching per site across `hosted::show`'s same-arity concrete
/// candidates. This is the shape every migrated program uses, and it works
/// only because `resolve::is_operator_dispatch_name` no longer lists `.`:
/// with the entry left in, the import stays unrewritten and every call is
/// `unknown word '.'`.
#[test]
fn a_bare_dot_dispatches_per_call_site_after_one_selective_import() {
    let t = Tree::new("bare-dot-import");
    let stdout = build_and_run_hosted(
        &t,
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: hosted::show | . | ;\n\
         : main ( -- ) 42 . -7 . \"hi\" . True . 100000 >usize . ;\n",
    );
    assert_eq!(stdout, "42\n-7\nhitrue\n100000\n");
}

/// R17b: the str dot writes all `len` bytes. Today's `%.*s` printf row stopped
/// at the first NUL, an artifact of C; Sooth's `len` deliberately counts
/// embedded NULs (`tests/phase3_strings.rs`'s
/// `interior_nul_diverges_sooth_len_from_c_strlen_native`). The `cstr` dot
/// keeps the terminator-bound semantics, so the same literal prints 5 bytes
/// one way and 2 the other -- the divergence made visible in one program.
#[test]
fn the_str_dot_writes_every_len_byte_and_the_cstr_dot_stops_at_the_nul() {
    let t = Tree::new("interior-nul");
    let stdout = build_and_run_hosted(
        &t,
        "import: intrinsics * ;\n\
         import: hosted::show | . | ;\n\
         : main ( -- ) \"ab\\0cd\" . \"\\n\" . \"ab\\0cd\" cstr . \"\\n\" . ;\n",
    );
    assert_eq!(stdout, "ab\0cd\nab\n");
}

/// R6/probe P6: a type with no dot is a located "no overload" error naming the
/// candidates, *with the import present* -- so this is the missing-candidate
/// diagnostic, not the missing-import one. An array is the case that matters:
/// P7.S3c ruled a view unprintable (an element loop and a separator policy are
/// a library word's decision), and that ruling now lives in the library's
/// candidate list rather than in a checker allowlist.
#[test]
fn a_type_with_no_dot_is_a_no_overload_error_naming_the_candidates() {
    let t = Tree::new("no-overload");
    let err = build_error(
        &t,
        "import: intrinsics * ;\n\
         import: hosted::show | . | ;\n\
         : main ( -- ) 0 4 fill . ;\n",
    );
    assert!(
        err.contains("no overload of `.` in `main`"),
        "unexpected diagnostic: {err}"
    );
    assert!(
        err.contains("candidate: `i64`") && err.contains("candidate: `str`"),
        "the diagnostic lists the printable types: {err}"
    );
}

/// R6/probe P6, the struct sibling: the forced deletions dropped
/// `check_struct_print_is_error`'s coverage that a struct has no `.`
/// overload; this restores it under the new no-overload diagnostic, with
/// the import present so it is the missing-candidate error, not the
/// missing-import one.
#[test]
fn a_struct_with_no_dot_is_a_no_overload_error_naming_the_candidates() {
    let t = Tree::new("no-overload-struct");
    let err = build_error(
        &t,
        "import: intrinsics * ;\n\
         import: hosted::show | . | ;\n\
         type: Point x i64 y i64 ;\n\
         : main ( -- ) 1 2 Point . ;\n",
    );
    assert!(
        err.contains("no overload of `.` in `main`"),
        "unexpected diagnostic: {err}"
    );
    assert!(
        err.contains("candidate: `i64`") && err.contains("candidate: `str`"),
        "the diagnostic lists the printable types: {err}"
    );
}

/// R6/probe P6, the enum sibling: the forced deletions dropped
/// `check_enum_print_is_error`'s coverage that an enum has no `.` overload;
/// this restores it the same way as the struct sibling above.
#[test]
fn an_enum_with_no_dot_is_a_no_overload_error_naming_the_candidates() {
    let t = Tree::new("no-overload-enum");
    let err = build_error(
        &t,
        "import: intrinsics * ;\n\
         import: hosted::show | . | ;\n\
         type: Shade | Red | Green | Blue ;\n\
         : main ( -- ) Red . ;\n",
    );
    assert!(
        err.contains("no overload of `.` in `main`"),
        "unexpected diagnostic: {err}"
    );
    assert!(
        err.contains("candidate: `i64`") && err.contains("candidate: `str`"),
        "the diagnostic lists the printable types: {err}"
    );
}

/// R3': no shim. A program that prints without the import gets the ordinary
/// unknown-word error -- not a builtin dispatch, and not the `intrinsics`
/// import hint (`.` left `is_name_dispatched_builtin`, so the gate no longer
/// covers it and would otherwise point at the wrong module).
#[test]
fn printing_without_the_import_is_an_unknown_word() {
    let t = Tree::new("no-import");
    let err = build_error(&t, "import: intrinsics * ;\n: main ( -- ) 42 . ;\n");
    assert!(
        err.contains("unknown word `.` in `main`"),
        "unexpected diagnostic: {err}"
    );
    assert!(
        !err.contains("import: intrinsics"),
        "the `intrinsics` gate must not claim `.`: {err}"
    );
}

/// The `SPY_DEF` split's run half (spec's migration row): the printing drop
/// overload still runs through the library dot and still emits the same
/// witness transcript the drop-order goldens pin. The check-only consumers got
/// a silent variant instead, since their in-process seed is `core`-only.
#[test]
fn a_printing_drop_overload_still_witnesses_drop_order() {
    let t = Tree::new("spy-witness");
    let stdout = build_and_run_hosted(
        &t,
        "import: intrinsics * ;\n\
         import: hosted::show | . | ;\n\
         type: Spy tag i64 ;\n\
         : drop ( Spy -- ) | s | \"drop \" . s Spy> . ;\n\
         : main ( -- ) 1 Spy drop 2 Spy drop ;\n",
    );
    assert_eq!(stdout, "drop 1\ndrop 2\n");
}
