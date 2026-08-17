//! Phase 7 Slice 3a (phase 2) goldens: a poly word instantiating and
//! constructing a generic type applied to its own type variable, ground
//! on demand at check/lowering time rather than only when parse time
//! already saw a fully-concrete application.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch single-file program, removed on drop (`tests/symbol_hijack.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3a-{}-{tag}-{seq}", std::process::id()));
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

/// Build and run the program, returning `(binary path, stdout, exit code)`. A
/// build failure panics: these programs are all well-typed, so a build error
/// is itself the regression to surface.
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

/// The declarations shared by every test here: a generic `Result` over its
/// own two type variables, and `reorder`, the brief's own probe word --
/// consuming `Result['T 'E]`, producing `Result['T 'E] 'T`.
const RESULT_AND_REORDER: &str = "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
     : reorder ( 'T Result['T 'E] -- Result['T 'E] 'T ) swap ;\n";

/// T1: `reorder` is instantiated at two **asymmetric** concrete pairs
/// (`[i64 str]` and its swap `[str i64]`); printing a value that is only
/// correct if the two monomorphs are tracked independently and positionally
/// -- `Result[i64 i64]` cannot tell `Ok 'T | Err 'E` from its swap, so this is
/// the shape that actually proves it.
#[test]
fn poly_word_consuming_result_over_its_own_vars_runs_at_two_asymmetric_instantiations() {
    let src = format!(
        "{RESULT_AND_REORDER}\
         : show_is ( Result[i64 str] -- ) | Ok |v| v . | Err |e| e . ;\n\
         : show_si ( Result[str i64] -- ) | Ok |v| v . | Err |e| e . ;\n\
         : main ( -- )\n\
           1 \"boom\" Err reorder . show_is\n\
           \"one\" 2 Err reorder . show_si ;\n"
    );
    let prog = Scratch::write("t1", &src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "1\nboomone2\n",
        "each monomorph must carry its own argument order: the first line \
         reorders `1`/`Err(\"boom\")` (prints `1`, then the Err payload \
         `boom` via show_is); the second reorders `\"one\"`/`Err(2)` \
         (prints `one`, then `2` via show_si)"
    );
}

/// T2: the two instantiations above mint two *distinct* mangled symbols, the
/// project's symmetric-instantiation-placebo precedent (`Result[i64 i64]`
/// cannot distinguish `Ok`/`Err`'s order, so the probe is asymmetric on
/// purpose) proven by `nm` over the built object, mirroring
/// `tests/symbol_hijack.rs`'s own `nm` pattern.
#[test]
fn two_asymmetric_instantiations_mint_distinct_symbols_nm() {
    let src = format!(
        "{RESULT_AND_REORDER}\
         : show_is ( Result[i64 str] -- ) drop ;\n\
         : show_si ( Result[str i64] -- ) drop ;\n\
         : main ( -- )\n\
           1 \"boom\" Err reorder drop show_is\n\
           \"one\" 2 Err reorder drop show_si ;\n"
    );
    let prog = Scratch::write("t2", &src);
    let binary = driver::build(prog.path()).expect("program should build");
    let nm = std::process::Command::new("nm")
        .arg(&binary)
        .output()
        .expect("nm should run");
    std::fs::remove_file(&binary).ok();
    let symbols = String::from_utf8_lossy(&nm.stdout);
    assert!(
        symbols.contains("sooth_mono_reorder__m0__t0_i64_t1_str"),
        "the [i64 str] instantiation's own symbol; nm found:\n{symbols}"
    );
    assert!(
        symbols.contains("sooth_mono_reorder__m0__t0_str_t1_i64"),
        "the [str i64] instantiation's own, distinct symbol; nm found:\n{symbols}"
    );
}

/// T3: the load-bearing test for R2/R3 together -- a poly word constructs
/// `Result['T i64]` at an instantiation (`Result[bool i64]`) no other
/// signature in the program names, so it can only exist through a downstream
/// mint (R2) reached through a poly body's own construction (R3). This is
/// the test that does not exist without option B (both in this slice).
#[test]
fn poly_word_constructs_a_monomorph_no_other_site_materializes() {
    let src = format!(
        "{RESULT_AND_REORDER}\
         : wrap ( 'T -- Result['T i64] ) Ok ;\n\
         : show ( Result[bool i64] -- ) | Ok |v| v . | Err |e| e . ;\n\
         : main ( -- ) true wrap show ;\n"
    );
    let prog = Scratch::write("t3", &src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(stdout, "true\n");
}

/// T-nontail: a poly body constructs a generic value and then moves a
/// *different* output into tail position (`swap` after `Ok`), so the
/// constructed value is not in 1:1 tail position -- proving the exit-time
/// `unify_poly_input` backstop (R3's soundness argument) actually fires
/// against the reordered residual stack, rather than being assumed true of
/// whatever `poly_call_term` just pushed.
#[test]
fn poly_body_constructor_off_tail_position_unifies_at_exit() {
    let src = format!(
        "{RESULT_AND_REORDER}\
         : mk ( 'T -- i64 Result['T i64] ) Ok 42 swap ;\n\
         : show ( i64 Result[bool i64] -- ) | Ok |v| v . drop | Err |e| e . drop ;\n\
         : main ( -- ) true mk show ;\n"
    );
    let prog = Scratch::write("t-nontail", &src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(stdout, "true\n");
}

/// T4 (R5.1): a generic applied to a type variable at nesting depth > 1
/// (`Box[Box['T]]`) is the out-of-scope rejection (D5) -- v1's boundary is
/// enforced, not merely assumed.
#[test]
fn generic_nested_depth_two_is_error() {
    let src = "type: Box 'T | Box 'T ;\n\
               : wrap ( 'T -- Box[Box['T]] ) Box ;\n\
               : main ( -- ) 1 wrap drop ;\n";
    let tokens = sooth::lexer::lex(src).unwrap();
    let err = sooth::parser::parse(&tokens).unwrap_err();
    assert!(
        err.contains("nesting depth"),
        "names the depth-2 rejection: {err}"
    );
}

/// T5 (R5.2): a constructor call in a poly body whose generic arguments are
/// not fully determined by its operands or by the declared output slot is a
/// located error naming the constructor and the undetermined variable.
#[test]
fn generic_constructor_undetermined_argument_is_error() {
    let src = "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
               : bad ( 'T i64 -- 'T ) Err drop ;\n\
               : main ( -- ) 1 2 bad drop ;\n";
    let tokens = sooth::lexer::lex(src).unwrap();
    let mut module = sooth::parser::parse(&tokens).unwrap();
    let err = sooth::check::check(&mut module).unwrap_err();
    assert!(
        err.contains("leaves the type variable `'T` undetermined"),
        "{err}"
    );
}

/// T6 (R5.3): a constructor operand whose type does not match the header's
/// declared payload is caught at the constructor call during body check,
/// never deferred into synthesis.
#[test]
fn generic_constructor_operand_mismatch_is_error() {
    let src = "type: Pair 'T val1 'T val2 'T ;\n\
               : mk ( 'T -- Pair['T] ) 1 swap Pair ;\n\
               : main ( -- ) \"oops\" mk drop ;\n";
    let tokens = sooth::lexer::lex(src).unwrap();
    let mut module = sooth::parser::parse(&tokens).unwrap();
    let err = sooth::check::check(&mut module).unwrap_err();
    assert!(err.contains("type mismatch in `mk`"), "{err}");
}

/// T7 (R5.4, D5): `dup`/`over` on a variable-bearing generic slot is
/// rejected -- a generic over variables is conservatively linear (never
/// `Copy`), consistent with the linear spine.
#[test]
fn dup_on_variable_bearing_generic_slot_is_error() {
    let src = "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
               : dupit ( 'T Result['T 'E] -- Result['T 'E] Result['T 'E] 'T ) dup ;\n\
               : main ( -- ) 1 2 Err dupit drop drop drop ;\n";
    let tokens = sooth::lexer::lex(src).unwrap();
    let mut module = sooth::parser::parse(&tokens).unwrap();
    let err = sooth::check::check(&mut module).unwrap_err();
    assert!(
        err.contains("cannot `dup` a generic type applied to a variable"),
        "{err}"
    );
}
