//! P7b.S7 exit goldens: quotation effects over type constructors (the `call`
//! extension). Phase 1 lifts fence #1 (`member_shape_is_supported`'s
//! Quotation arm) so a trait-var-headed `App` inside a member quotation row
//! parses to a `TraitDecl` with the declared effect intact -- narrowly: an
//! App headed by anything but the trait's own type variable stays rejected.
//! Harness style from `tests/phase7b_slice2.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs7-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

fn build_ok(entry: &Path) {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    std::fs::remove_file(&binary).ok();
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", &format!("import: intrinsics * ;\n{src}"));
    (t, entry)
}

/// The hosted twin (`tests/phase7b_slice2.rs`'s pattern): a bare package
/// cannot import `core`, and printing needs `hosted::show`'s `.`.
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs7 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

/// Build, run, and keep the binary: `(binary, stdout)`. The binary is kept
/// (not deleted) so IR/symbol assertions can still read it.
fn build_run_keep(tag: &str, src: &str) -> (Tree, PathBuf, String) {
    let (t, entry) = single_file_hosted(tag, src);
    let build = sooth_build(&entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the binary should run");
    assert_eq!(
        run.status.code(),
        Some(0),
        "the built binary should exit 0; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    (t, binary, stdout)
}

/// `binary`'s symbol *names*: `nm`'s last whitespace-separated field per
/// line (`tests/phase7b_slice3.rs`'s convention -- a split field, not a
/// substring `contains()`, so a typo or a longer embedding symbol can't
/// satisfy the assertion).
fn symbols(binary: &Path) -> Vec<String> {
    let nm = Command::new("nm")
        .arg(binary)
        .output()
        .expect("nm should run");
    let text = String::from_utf8_lossy(&nm.stdout).into_owned();
    text.lines()
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .collect()
}

/// G1: `monad_bind_declares_app_in_quotation_row` -- `bind`'s `'F['T]` input
/// and the row-nested `'F['U]` output inside its quotation parameter both
/// apply the trait's own `'F`; pre-REQ-1 the row-nested one hit
/// `app_in_member_quotation_row_error` (exit 1), post-REQ-1 the whole
/// declaration parses to a `TraitDecl` with the effect intact.
#[test]
fn monad_bind_declares_app_in_quotation_row() {
    let src = "\
trait: Monad['F: * -> *] :
  bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("g1-monad-bind", src);
    build_ok(&entry);
}

/// G4a (regression pin, reclassified per the paper tests): a bare `'F` used
/// as a plain type in a quotation row's output, after an earlier App-head
/// mention establishes its kind as `* -> *`, is a kind error
/// (`arrow_var_used_bare_error`) that fires independently of, and earlier
/// than, the row-shape gate -- a placebo for the row-fence's narrowness (it
/// would die identically under a hypothetical blanket admit), kept only as
/// a regression pin that the independent kind check still fires.
#[test]
fn kind_incorrect_app_in_quotation_row_is_error() {
    let src = "\
trait: Bad['F: * -> *] :
  m ( 'F['T] [ 'T -- 'F ] -- 'F['T] ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("g4a-bare-kind-error", src);
    let err = build_error(&entry);
    assert!(
        err.contains("is used as a plain type but has kind `* -> *`"),
        "{err}"
    );
    // Located at the bare `'F` inside the row (line 3, col 22), not just
    // named -- pins REQ-4's "located, not silent".
    assert!(err.contains("line 3, col 22"), "{err}");
}

/// G4b (the real row-fence-narrowness witness): `'G` is a member local, not
/// the trait's own header variable, heading an application inside the
/// quotation's output row. Still rejected post-REQ-1, via the same
/// `app_in_member_quotation_row_error` dispatch -- `member_shape_is_supported`'s
/// own `App` arm (`head == 0` only) still rejects `'G`'s application when the
/// Quotation arm's recursion reaches it, so the row admits only an App headed
/// by the trait's own variable. The message is the REQ-3-corrected one, not
/// the pre-S7 "keep quotation rows App-free" text (no longer true in general).
#[test]
fn member_local_headed_app_in_quotation_row_is_still_unsupported() {
    let src = "\
trait: Bad2['F: * -> *] :
  m ( 'F['T] [ 'T -- 'G['U] ] -- 'F['T] ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("g4b-local-headed-row", src);
    let err = build_error(&entry);
    assert!(
        err.contains("applies a type variable inside a quotation row"),
        "{err}"
    );
    assert!(
        err.contains(
            "an App inside a quotation row must be headed by the trait's own type variable"
        ),
        "{err}"
    );
    // Located at the member (`m`, line 3, col 3), not just named -- pins
    // REQ-4's "located, not silent" (the retired slice2 test's `line 3, col
    // 3` pattern).
    assert!(err.contains("line 3, col 3"), "{err}");
}

/// Phase 3 (REQ-6/REQ-7/REQ-11): `Monad.bind` dispatches per constructor and
/// splices with zero frame. `bind` is declared `inline`, mirroring
/// `tests/phase7b_slice3.rs`'s `Sized['S] : size inline (...)` pattern -- a
/// non-inline member mints a `bind;Monad;...` symbol (S3's convention), so
/// pinning `nm` finds none of it proves the splice, not just that the value
/// came out right.
fn monad_trait_decl() -> &'static str {
    "trait: Monad['F: * -> *] :\n  \
     bind inline ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] ) ;\n\
     ;\n"
}

/// G2: `option_bind_dispatches_and_short_circuits`. Both `impl: Monad`
/// blocks (Option and Result) live in this one program, so an impl-blind
/// dispatcher that always picked the sole candidate cannot pass: `4 Some
/// [ half ] bind` (even) -> `Some(2)`; `nothing [ half ] bind` (the input is
/// already `None`) -> `None`, short-circuiting the quotation entirely --
/// `showopt`'s `None` arm now prints a distinguishable `-1` instead of a
/// silent `drop`, so that arm is observed, not just "didn't crash". A
/// third call routes through the *other* impl (`Ok 4 [ half_result ] bind`)
/// to prove this program's dispatch actually keys off the constructor, not
/// off there being only one candidate in scope. The explicit instantiation
/// (`bind[i64 i64]`) is the same mono-dispatch spelling
/// `tests/phase7b_slice4.rs`'s `map[i64 i64]` golden already uses -- a
/// concrete (non-generic) caller needs `'U` supplied, since nothing in the
/// call's own operands binds it (measured this phase; unrelated to REQ-11,
/// which is the poly-caller path).
#[test]
fn option_bind_dispatches_and_short_circuits() {
    let src = format!(
        "\
    import: core::prelude * ;\n\
    import: core::option * ;\n\
    import: core::result * ;\n\
    {trait}\
    impl: Monad for Option\n  \
    : bind swap ~[ ( Some ) Some> swap call ] ~[ ( None ) drop drop None ] Option? ;\n\
    ;\n\
    impl: Monad for Result\n  \
    : bind swap ~[ ( Ok ) Ok> swap call ] ~[ ( Err ) Err> swap drop Err ] Result? ;\n\
    ;\n\
    : half ( i64 -- Option[i64] )\n  \
    dup 2 mod 0 eq ~[ 1 shr Some ] ~[ drop None ] if ;\n\
    : half_result ( i64 -- Result[i64 str] )\n  \
    dup 2 mod 0 eq ~[ 1 shr Ok ] ~[ drop \"odd\" Err ] if ;\n\
    : nothing ( -- Option[i64] ) None ;\n\
    : showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop -1 . ] Option? ;\n\
    : showres ( Result[i64 str] -- ) ~[ ( Ok ) Ok> . ] ~[ ( Err ) Err> . ] Result? ;\n\
    : main ( -- )\n  \
    4 Some [ half ] bind[i64 i64] showopt\n  \
    nothing [ half ] bind[i64 i64] showopt\n  \
    4 Ok [ half_result ] bind[i64 str i64] showres ;\n",
        trait = monad_trait_decl()
    );
    let (_t, binary, stdout) = build_run_keep("g2-option-bind", &src);
    // Some/2 -> 2; None short-circuit -> -1; Result/Ok dispatch -> 2.
    assert_eq!(stdout, "2\n-1\n2\n");

    // IR: no frame beyond a hand-written `and_then` -- `bind` is `inline`,
    // so its member symbol (`bind;Monad;...` or a `bind__m0`/mono form) must
    // not appear in the binary at all.
    let syms = symbols(&binary);
    let bind_syms: Vec<&String> = syms.iter().filter(|s| s.contains("bind")).collect();
    assert!(
        bind_syms.is_empty(),
        "an inline member mints no symbol; nm found: {bind_syms:?}"
    );
}

/// G3: `result_bind_dispatches_and_short_circuits_on_err`. Both `impl:
/// Monad` blocks live in this one program too (Result and Option), again
/// ruling out a sole-candidate fallthrough: `Ok 4 [ half_result ] bind`
/// (even) -> `Ok 2`; `boom [ half_result ] bind` (the input is already
/// `Err "boom"`) -> `Err "boom"`, short-circuiting the quotation entirely
/// and printing the distinguishable `"boom"` string (not `half_result`'s
/// own `"odd"`), so the `Err` arm itself -- not the callee -- is what's
/// observed. A third call routes through the Option impl to confirm
/// constructor-keyed dispatch in the presence of both candidates.
#[test]
fn result_bind_dispatches_and_short_circuits_on_err() {
    let src = format!(
        "\
    import: core::prelude * ;\n\
    import: core::result * ;\n\
    import: core::option * ;\n\
    {trait}\
    impl: Monad for Result\n  \
    : bind swap ~[ ( Ok ) Ok> swap call ] ~[ ( Err ) Err> swap drop Err ] Result? ;\n\
    ;\n\
    impl: Monad for Option\n  \
    : bind swap ~[ ( Some ) Some> swap call ] ~[ ( None ) drop drop None ] Option? ;\n\
    ;\n\
    : half_result ( i64 -- Result[i64 str] )\n  \
    dup 2 mod 0 eq ~[ 1 shr Ok ] ~[ drop \"odd\" Err ] if ;\n\
    : half ( i64 -- Option[i64] )\n  \
    dup 2 mod 0 eq ~[ 1 shr Some ] ~[ drop None ] if ;\n\
    : boom ( -- Result[i64 str] ) \"boom\" Err ;\n\
    : showres ( Result[i64 str] -- ) ~[ ( Ok ) Ok> . ] ~[ ( Err ) Err> . ] Result? ;\n\
    : showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop -1 . ] Option? ;\n\
    : main ( -- )\n  \
    4 Ok [ half_result ] bind[i64 str i64] showres\n  \
    boom [ half_result ] bind[i64 str i64] showres\n  \
    4 Some [ half ] bind[i64 i64] showopt ;\n",
        trait = monad_trait_decl()
    );
    let (_t, binary, stdout) = build_run_keep("g3-result-bind", &src);
    // Ok/2 -> 2; Err short-circuit -> "boom" (str `.` has no trailing
    // newline); Option/Some dispatch -> 2.
    assert_eq!(stdout, "2\nboom2\n");

    let syms = symbols(&binary);
    let bind_syms: Vec<&String> = syms.iter().filter(|s| s.contains("bind")).collect();
    assert!(
        bind_syms.is_empty(),
        "an inline member mints no symbol; nm found: {bind_syms:?}"
    );
}

/// Reg: `named_ctor_of_quotation_argument_unchanged` (probe P5's shape,
/// `Box[[i64 -- i64]]`). This phase ships **zero** production code changes
/// -- the REQ-11 measurement (see `unify_member_operand_rejects_a_literal_
/// quotation_operand` in `src/check/poly.rs`) found a real gap but the
/// permissive fallback arm was reverted, so `unify_member_operand` is
/// byte-identical to HEAD. This fixture's validity as a regression pin
/// comes from having no `trait:`/`impl:` at all: a plain named-constructor
/// application of a quotation type structurally cannot reach
/// `unify_member_operand` (that fn is only ever called from trait-member
/// dispatch), so it builds, runs, and prints the value the boxed quotation
/// produces regardless of anything this phase did or didn't touch.
#[test]
fn named_ctor_of_quotation_argument_unchanged() {
    let src = "\
type: Box['T] v 'T ;\n\
: h ( Box[ [ i64 -- i64 ] ] -- i64 ) Box> 5 swap call ;\n\
: main ( -- ) [ 1 add ] Box h . ;\n";
    let (_t, _binary, stdout) = build_run_keep("reg-named-ctor-quotation", src);
    assert_eq!(stdout, "6\n");
}
