//! P7.S11 goldens: an unbounded `inline` combinator's standalone check can
//! ground a generic type its own declared output (or a nested slot -- a
//! quotation-input effect, an array element) applies, without leaking a
//! scratch-minted monomorph or shape into the live registries (R1/R2/R2.1/
//! R3/R6, phase 1). Phase 2 (R4) additionally registers the grounded
//! monomorph's generated constructor/destructure sigs into a word-scoped env
//! copy, so a body term naming one of those variants resolves -- lifting the
//! def-site gate all the way to end-to-end, for a combinator whose header
//! already has a parse-time monomorph in the program. The frozen call-site
//! env (a check-time-only mint never reaching a *later* concrete caller
//! without one) is a separate, pre-existing gap this slice does not fix; see
//! the brief.

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

/// Golden 1: the end-to-end exit dogfood, with a parse-time sibling
/// monomorph (`mki`). Because `mki` grounds `Result[i64 i64]` at parse time,
/// the standalone check's `apply_subst` hits `lookup_enum`'s dedup and
/// **nothing is minted**: `local.enums == enums`, so R4's body never runs.
/// This golden witnesses R1 and R3 only -- it is not an R2-mint, R4 or
/// sig-generation witness (golden 6 carries no sibling and is that witness).
/// `'U` is deliberately collapsed to `'T`: see golden 5.
#[test]
fn unbounded_combinator_constructing_its_generic_output_builds_and_runs() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : mki ( i64 -- Result[i64 i64] ) Ok ;\n\
         : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;\n\
         : main ( -- )\n\
           7 ~[ 1 add ] wrap\n\
           ~[ ( Ok ) Ok> . ]\n\
           ~[ ( Err ) Err> . ]\n\
           Result? ;\n";
    let prog = Scratch::write("golden1", src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "8\n");
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

/// Golden 5: pins the boundary that made golden 1 collapse `'U` to `'T`. With
/// the original `( 'T ~[ 'T -- 'U ] -- Result['U i64] )` shape the def-site
/// error is gone (R1/R2/R4 ground it fine), but the call site reports an
/// output-only variable no input binds, and the note's suggested remedy
/// (`wrap[i64 i64]`) does not apply either: a combinator is not a
/// polymorphic word and takes no explicit type arguments.
#[test]
fn an_output_only_type_variable_is_still_uncallable() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : wrap inline ( 'T ~[ 'T -- 'U ] -- Result['U i64] ) call Ok ;\n\
         : main ( -- ) 7 ~[ 1 add ] wrap drop ;\n";
    let prog = Scratch::write("golden5", src);
    let err = build_error(prog.path());
    assert!(
        err.contains("has output variable `'U` that no input binds"),
        "expected the unbound-output-variable error, got: {err}"
    );

    let explicit_src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : wrap inline ( 'T ~[ 'T -- 'U ] -- Result['U i64] ) call Ok ;\n\
         : main ( -- ) 7 ~[ 1 add ] wrap[i64 i64] drop ;\n";
    let explicit_prog = Scratch::write("golden5-explicit", explicit_src);
    let explicit_err = build_error(explicit_prog.path());
    assert!(
        explicit_err.contains(
            "takes no type arguments; only a call to a polymorphic word may be explicitly instantiated"
        ),
        "expected the no-type-arguments-on-a-combinator error, got: {explicit_err}"
    );
}

/// Golden 6 (P7.S11-follow, Part 1/4): the check-time monomorph's
/// constructor/accessor now resolves at the call site. Golden 1's source
/// minus the `mki` line has no parse-time sibling monomorph, so `wrap`'s
/// own signature grounding mints `Ok`'s constructor check-time; Part 1
/// forces that mint ahead of the body splice and Part 4's `env`-miss
/// fallback resolves it in `main`, so the whole program builds and runs.
/// `main` is golden 1's minus `mki`, so it prints the same value.
#[test]
fn a_check_time_monomorphs_constructors_resolve_at_the_call_site() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;\n\
         : main ( -- )\n\
           7 ~[ 1 add ] wrap\n\
           ~[ ( Ok ) Ok> . ]\n\
           ~[ ( Err ) Err> . ]\n\
           Result? ;\n";
    let prog = Scratch::write("golden6", src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0, "expected the program to build and run cleanly");
    assert_eq!(stdout, "8\n");
}

/// Golden 7: R4's reach boundary. A combinator whose declared output grounds
/// `Result` but whose body also constructs a `Pair['T]` the signature never
/// mentions must still fail on that unrelated header: R4 registers only the
/// generated sigs of the monomorph the *signature grounding* minted, not
/// every construction the body happens to perform.
#[test]
fn an_intermediate_construction_over_a_different_header_is_still_unknown() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         type: Pair['A] | Nil | One 'A ;\n\
         : mki ( i64 -- Result[i64 i64] ) Ok ;\n\
         : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call dup One drop Ok ;\n\
         : main ( -- )\n\
           7 ~[ 1 add ] wrap\n\
           ~[ ( Ok ) Ok> . ]\n\
           ~[ ( Err ) Err> . ]\n\
           Result? ;\n";
    let prog = Scratch::write("golden7", src);
    let err = build_error(prog.path());
    assert!(
        err.contains("unknown word `One` in `wrap`"),
        "expected the unrelated header's constructor to stay unknown: {err}"
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

/// Golden 9 (P7.S11-follow): the mutation-4 witness at integration level. An
/// earlier monomorphic word (`boxit`, spliced through `seed`) grounds a
/// *different* generic header at check time and flushes it, leaving
/// `enum_base` stale by exactly that batch's size -- word order in source is
/// load-bearing: `seed` precedes `wrap` in the single word loop, so its
/// flush happens first. The standalone combinator check must still land its
/// own scratch mint at the correct id (P0-A's rebase), and Part 4's
/// id-derivation (which relies on that same base bookkeeping) must still
/// resolve `Ok`'s constructor at the call site, so the whole program builds
/// and runs rather than panicking on a wrong id. `main` (`seed 7 ~[ 1 add ]
/// wrap drop`) has no eliminator and no `.`, so it prints nothing.
#[test]
fn a_standalone_mint_after_an_earlier_check_time_mint_lands_at_the_right_id() {
    let src = "type: Box['A] | Empty | Full 'A ;\n\
         type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : boxit ( 'T -- Box['T] ) Full ;\n\
         : seed ( -- ) 1 boxit drop ;\n\
         : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;\n\
         : main ( -- ) seed 7 ~[ 1 add ] wrap drop ;\n";
    let prog = Scratch::write("golden9", src);
    let (_, code) = build_and_run(prog.path());
    assert_eq!(code, 0, "expected the program to build and run cleanly");
}

/// Golden 10 (P7.S11-follow): R4's *struct* half, otherwise unwitnessed by
/// goldens 1-9 (every prior fixture mints an enum). `wrap` is `inline`, so
/// this also exercises the splice-time recheck of its body in `main` -- with
/// the struct twin's decl-fallback and constructor fallback wired, `main`'s
/// `Cell` call resolves through the same check-time mint (the same
/// frozen-call-site gap golden 6 pins for enums), so the whole program
/// builds and runs. `main` is `… wrap drop` with no `.`, so it prints
/// nothing.
#[test]
fn a_check_time_struct_monomorphs_constructor_resolves_at_the_call_site() {
    let src = "type: Cell['T] val 'T ;\n\
         : wrap inline ( 'T ~[ 'T -- 'T ] -- Cell['T] ) call Cell ;\n\
         : main ( -- ) 7 ~[ 1 add ] wrap drop ;\n";
    let prog = Scratch::write("golden10", src);
    let (_, code) = build_and_run(prog.path());
    assert_eq!(code, 0, "expected the program to build and run cleanly");
}

/// Golden 6b (P7.S11-follow): the minimal, unconfounded control for Part 4's
/// constructor fallback and Part 2's enum drop/layout reads -- enum
/// construction + `drop`, no eliminator at all. Golden 6 routes through the
/// eliminator (`Result?`); golden 9 has no eliminator either but layers a
/// stale-`enum_base` confound on top (the `Box`/`boxit`/`seed` prefix mints
/// `Box[i64]` before `wrap`'s mint), so a failure there cannot discriminate
/// the constructor-fallback mechanism from the base bookkeeping. This one
/// header, one mint, no eliminator and no stale-base interaction.
#[test]
fn a_check_time_enum_monomorph_constructs_and_drops_without_an_eliminator() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;\n\
         : main ( -- ) 7 ~[ 1 add ] wrap drop ;\n";
    let prog = Scratch::write("golden6b", src);
    let (_, code) = build_and_run(prog.path());
    assert_eq!(code, 0, "expected the program to build and run cleanly");
}

/// A `dup` witness for the extended-slice `is_copy` wiring (P7.S11-follow,
/// Part 2). Parts 1/4 make it possible to `dup` a check-time-only monomorph,
/// which reaches the unguarded `is_copy` read at `src/check.rs`'s `dup` site
/// (an index past `ctx.structs()`/`ctx.enums()`, panicking without the
/// extended-slice wiring). `Result[i64 i64]` is all-`Copy`, so `dup` here is
/// legal.
#[test]
fn a_check_time_monomorph_survives_dup_without_a_panic() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;\n\
         : main ( -- ) 7 ~[ 1 add ] wrap dup drop drop ;\n";
    let prog = Scratch::write("golden-dup", src);
    let (_, code) = build_and_run(prog.path());
    assert_eq!(code, 0, "expected the program to build and run cleanly");
}

/// A bind witness for the extended-slice `is_linear` wiring (P7.S11-follow,
/// Part 2) -- a second, independent hole `dup` alone does not cover.
/// `is_linear` is a thin wrapper over `is_copy` and inherits the identical
/// unbounded read, but is reached by an entirely different path: binding a
/// check-time monomorph to a local (`| r |`), with no `dup`/`over` in sight.
/// `is_copy` and `is_linear` are called from disjoint sites (`check.rs` vs.
/// `terms.rs`), so this golden and the `dup` golden above are independent
/// witnesses -- fixing one does not imply the other is fixed.
#[test]
fn a_check_time_monomorph_survives_a_local_bind_without_a_panic() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
         : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;\n\
         : main ( -- ) 7 ~[ 1 add ] wrap | r | r drop ;\n";
    let prog = Scratch::write("golden-bind", src);
    let (_, code) = build_and_run(prog.path());
    assert_eq!(code, 0, "expected the program to build and run cleanly");
}
