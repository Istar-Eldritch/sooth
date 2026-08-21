//! Phase 7 Slice 3b-follow goldens: a **row-typed inline combinator** consumed
//! in a **non-inline polymorphic** body, so a generic word can loop and branch
//! as a monomorphized function instead of forcing every call site to splice its
//! whole body.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch program directory, removed on drop (`tests/phase7_slice3b.rs`'s
/// own pattern). Multi-file here rather than single-file: `times` lives in
/// `lib/combinators.sth` and is reached by `import:`, which is also what makes
/// its name arrive at the call site *mangled* (`times__m1`) -- the spelling the
/// dispatch has to resolve.
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3bf-{}-{tag}-{seq}", std::process::id()));
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

/// An `import:` line for one of the shipped library files, by absolute path, so
/// a scratch program in the temp dir reaches the real `lib/`.
fn import_lib(file: &str, selective: &str) -> String {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("lib").join(file);
    format!("import: \"{}\" c | {selective} | ;\n", lib.display())
}

fn build_and_run(src: &Path) -> (String, i32) {
    let binary = driver::build(src).expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

fn build_err(tag: &str, src: &str) -> String {
    let scratch = Scratch::write(tag, src);
    driver::build(scratch.path()).expect_err("this program should be rejected")
}

/// Two user-written always-spliced combinators whose declared row is the same
/// on both sides, so the library is not the only witness for the mechanism and
/// a single-file program can exercise it. They differ only in how many times
/// they run the arm, which is what makes a run-count observable below.
const ONCE_AND_TWICE: &str = ": once  inline ( ..s ~[ ..s -- ..s ] -- ..s ) | f | f call ;\n\
     : twice inline ( ..s ~[ ..s -- ..s ] -- ..s ) | f | f call f call ;\n";

#[test]
fn times_in_a_non_inline_generic_body_compiles_and_runs() {
    // R2/R3, the non-shape-changing exit criterion: `times` declares
    // `~[ ..s i64 -- ..s ]`, its row grounds to the caller region below its
    // fixed inputs, and the arm is walked over that region. The word is
    // deliberately *not* `inline`, so it is monomorphized once per
    // instantiation rather than spliced per call site -- and it is called at
    // two types, so `'T` is carried rigidly through the loop rather than
    // coincidentally matching one instantiation.
    //
    // The arm swaps the two carried slots, so the result reads the *parity of
    // the iteration count* straight off the loop: an odd count leaves the pair
    // swapped and an even one does not. That is what makes these assertions
    // able to fail while still compiling -- a wrong iteration count or a wrong
    // exit-row join changes the printed value rather than the build.
    let src = format!(
        "{}\
         : lower ( 'T: Copy Ord 'T i64 -- 'T ) ~[ | i | swap ] times drop ;\n\
         : main ( -- )\n\
           5 7 3 lower .\n\
           5 7 2 lower .\n\
           2.5 3.5 1 lower .\n\
           2.5 3.5 0 lower . ;\n",
        import_lib("combinators.sth", "times")
    );
    let scratch = Scratch::write("times-body", &src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "7\n5\n3.5\n2.5\n");
    assert_eq!(code, 0);
}

#[test]
fn user_row_combinator_in_a_non_inline_generic_body_compiles_and_runs() {
    // R2: the dispatch is driven by the callee's declared `PolySig`, not by
    // name, so a *user* combinator with no library involvement reaches it too.
    // The two words differ only in their callee, and the same arm under each
    // prints a different value -- so the arm is genuinely run by the callee's
    // own body, once or twice, rather than checked and discarded.
    let src = format!(
        "{ONCE_AND_TWICE}\
         : odd  ( 'T: Copy Ord 'T -- 'T ) ~[ swap ] once  drop ;\n\
         : even ( 'T: Copy Ord 'T -- 'T ) ~[ swap ] twice drop ;\n\
         : main ( -- ) 5 7 odd . 5 7 even . 2.5 3.5 odd . 2.5 3.5 even . ;\n"
    );
    let scratch = Scratch::write("user-row", &src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "7\n5\n3.5\n2.5\n");
    assert_eq!(code, 0);
}

#[test]
fn single_arm_times_arm_deepening_the_row_is_error() {
    // R3, the soundness test (not a diagnostic test): a lone arm is checked
    // against the **pre-seeded grounded entry row**, not against a sibling.
    // The eliminator's cross-arm rule is seeded from the *first arm's* exit,
    // because an eliminator's arms are supposed to change the shape; with that
    // seeding `times` has nothing to compare against at all and this body --
    // whose arm leaves one more slot than it entered with, so the loop's
    // back-edge stack depth does not match its entry -- is wrongly accepted.
    let err = build_err(
        "times-deepens",
        &format!(
            "{}\
             : bad ( 'T: Copy Ord 'T i64 -- 'T ) ~[ dup ] times drop ;\n\
             : main ( -- ) 5 7 3 bad . ;\n",
            import_lib("combinators.sth", "times")
        ),
    );
    assert!(
        err.contains(
            "the quotation passed to `times` in `bad` (line 2) was declared `~[ ..s i64 -- ..s ]`, but it leaves `'T 'T i64 i64` where that requires `'T 'T`"
        ),
        "the arm must be held to the row it entered with: {err}"
    );
}

#[test]
fn single_arm_times_arm_consuming_the_row_is_error() {
    // R3, the other direction: an arm that eats a slot of the grounded row it
    // entered with. Both directions are pinned because a comparison written
    // one-sided (`exit.len() < want.len()`) passes one and not the other.
    let err = build_err(
        "times-consumes",
        &format!(
            "{}\
             : bad ( 'T: Copy Ord 'T i64 -- 'T ) ~[ | i | drop ] times ;\n\
             : main ( -- ) 5 7 3 bad . ;\n",
            import_lib("combinators.sth", "times")
        ),
    );
    assert!(
        err.contains("but it leaves `'T` where that requires `'T 'T`"),
        "an arm may not consume the row it rides on: {err}"
    );
}

#[test]
fn times_arm_leaving_a_rigid_variable_as_a_concrete_type_is_error() {
    // L1: the row is compared **structurally under rigid variables**. This
    // arm's exit is the same *depth* as the row it entered with, and differs
    // only in that a `'T` slot has become `i64` -- so it is caught by the
    // per-slot comparison and not by the length check above.
    let err = build_err(
        "times-rigid",
        &format!(
            "{}\
             : bad ( 'T: Copy Ord 'T i64 -- 'T ) ~[ | i | drop 1 ] times drop ;\n\
             : main ( -- ) 5 7 3 bad . ;\n",
            import_lib("combinators.sth", "times")
        ),
    );
    assert!(
        err.contains("but it leaves `'T i64` where that requires `'T 'T`"),
        "`'T` is never bound to the arm's `i64`: {err}"
    );
}

#[test]
fn narrowed_guard_keeps_call_branch_and_tag_located() {
    // R4: the three consumers this slice does *not* deliver keep their located
    // rejection. Each asserts the **exact message**: a bare "the build fails"
    // assertion passes identically against an `unknown word` fallthrough,
    // which is the one regression these exist to catch (nothing registers
    // these names on the poly path, so that is where they land if the guard
    // stops naming one).
    //
    // `tag`'s arm is reached the same way the other two are: the guard is
    // name-based, so an untagged quotation literal is enough to reach it.
    for (name, body) in [
        ("call", "~[ drop ] call"),
        ("branch", "over over gt ~[ drop ] ~[ swap drop ] branch"),
        ("tag", "~[ dup ] tag drop"),
    ] {
        let err = build_err(
            name,
            &format!(": bad ( 'T: Copy Ord 'T -- 'T ) {body} ;\n: main ( -- ) 5 7 bad . ;\n"),
        );
        assert!(
            err.contains(&format!(
                "error: `{name}` on a quotation in the polymorphic body of `bad` (line 1) is not yet supported"
            )),
            "`{name}` must stay a located rejection naming itself, not `unknown word`: {err}"
        );
    }
}

#[test]
fn shape_changing_combinator_is_still_located() {
    // OQ2/R3: a combinator whose input and output rows *differ* (`if`,
    // `unless`, and any user combinator declaring two rows) has no fixed exit
    // row for the declaration to require, so it is not part of the
    // non-shape-changing dispatch. It stays a located rejection here, and
    // `unless` -- which never reached the old name guard at all -- now reaches
    // this one, which is what proves the dispatch, not the guard, is what sees
    // it.
    for body in [
        "over over gt ~[ drop ] ~[ swap drop ] if",
        "over over gt ~[ drop ] ~[ swap drop ] unless",
    ] {
        let err = build_err(
            "shape-changing",
            &format!(": bad ( 'T: Copy Ord 'T -- 'T ) {body} ;\n: main ( -- ) 5 7 bad . ;\n"),
        );
        assert!(
            err.contains(
                "on a quotation in the polymorphic body of `bad` (line 1) is not yet supported"
            ) && err.contains("whose declared row is the same on both sides"),
            "a shape-changing row is deferred, located: {err}"
        );
    }
}

#[test]
fn inline_generic_body_still_splices_a_row_combinator() {
    // Regression: the whole `inline` route this slice exists to *avoid* forcing
    // is untouched. An `inline` generic word consuming `if` is spliced into its
    // concrete caller and grounds its row against a real stack
    // (`examples/poly_if.sth`'s shape), and that path does not go through the
    // poly walk at all.
    let src =
        ": mymax inline ( 'T: Copy Ord 'T -- 'T ) over over gt ~[ drop ] ~[ swap drop ] if ;\n\
               : main ( -- ) 2 9 mymax . 9.5 2.5 mymax . ;\n";
    let scratch = Scratch::write("inline-splice", src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "9\n9.5\n");
    assert_eq!(code, 0);
}
