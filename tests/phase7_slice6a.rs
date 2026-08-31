//! P7.S6a exit goldens: a generic header binding a length variable
//! (`type: Buffer['T 'N: Len] data array['T 'N] ;`) end to end -- header
//! parsing, substitution/instantiation of a length-carrying field, use-site
//! parsing, signature unification, and (R8b, this phase) impl-target
//! matching and specificity over a length-carrying header. Driven through
//! the real `sooth` binary, so the whole pipeline is exercised, not just
//! individual stages.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s6a-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(
            &path,
            format!("{contents}{}", common::printing_import(contents)),
        )
        .unwrap();
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
        .arg("--manifest")
        .arg(common::fixture_manifest())
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

fn build_and_run(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    std::fs::remove_file(&binary).ok();
    String::from_utf8_lossy(&run.stdout).into_owned()
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

const BUFFER_HEADER: &str = "type: Buffer['T 'N: Len] data array['T 'N] ;\n";

/// The `capacity` fixture the round-4 spec redesigns around: `'N` carried by
/// a **second, bare-array-typed parameter**, not field projection out of
/// the `Buffer` receiver (projecting into a generic receiver in a
/// non-inline body is rejected -- see `Buffer`'s own note in the design
/// text). This exercises the pre-existing generic-length-array `len`
/// machinery instead.
const CAPACITY_WORD: &str = "\
: capacity['T 'N: Len] ( array['T 'N] & Buffer['T 'N] -- usize )
    drop len swap drop
;\n";

/// R8a's exit dogfood: a header-carried length, constructed and called end
/// to end, pinned to the concrete length (`256`), not merely \"compiles\".
/// `mk_buffer` forces `Buffer[u8 256]`'s monomorph into existence via a
/// concrete signature (the ctor word `Buffer` is only reachable once some
/// concrete instantiation of the header is on record).
#[test]
fn buffer_header_with_length_parameter_builds_and_runs() {
    let src = format!(
        "{BUFFER_HEADER}\
         : mk_buffer ( array[u8 256] -- Buffer[u8 256] ) Buffer ;\n\
         {CAPACITY_WORD}\
         : main ( -- )
           0 >u8 256 fill |arr|
           0 >u8 256 fill mk_buffer |buf|
           arr &buf capacity .
           buf drop
         ;\n"
    );
    let (_t, entry) = single_file("dogfood", &src);
    let out = build_and_run(&entry);
    assert_eq!(out, "256\n");
}

/// R5: `Buffer[u8 256]` and `Buffer[u8 512]` are distinct monomorphs at
/// check time, not just in the symbol name -- a word declared over one
/// rejects a value built at the other. The mismatch surfaces at
/// construction (the ctor's own declared field type is `array[u8 256]`),
/// which already proves the two lengths mint genuinely different `Type`s.
#[test]
fn distinct_buffer_lengths_are_distinct_types() {
    let src = format!(
        "{BUFFER_HEADER}\
         : use256 ( Buffer[u8 256] -- ) drop ;\n\
         : main ( -- )
           0 >u8 512 fill Buffer |buf|
           buf use256
         ;\n"
    );
    let (_t, entry) = single_file("distinct", &src);
    let err = build_error(&entry);
    assert!(err.contains("array[u8 256]"), "{err}");
    assert!(err.contains("array[u8 512]"), "{err}");
}

/// R8a (round-4 review fix): the sibling of the golden above, but through a
/// poly-body **cross-call** rather than an ordinary call -- `sink` and
/// `caller` are both generic (`['T]`), so this reaches `poly_cross_match`'s
/// `Generic`/`Generic` arm specifically, not the plain field-type mismatch
/// the non-generic test above already covers. Before that arm compared
/// `len_args`, this exact program built and ran silently (printing nothing
/// useful and exiting 0), since a body declared over `Buffer['T 8]` could
/// pass its operand straight into a callee declared over `Buffer['T 4]`.
#[test]
fn poly_body_cross_call_rejects_a_mismatched_concrete_buffer_length() {
    let src = format!(
        "{BUFFER_HEADER}\
         : sink['T]   ( Buffer['T 4] -- ) drop ;\n\
         : bad_caller['T] ( Buffer['T 8] -- ) sink ;\n\
         : mk8 ( array[u8 8] -- Buffer[u8 8] ) Buffer ;\n\
         : main ( -- )
           0 >u8 8 fill mk8 bad_caller
         ;\n"
    );
    let (_t, entry) = single_file("cross-call-length", &src);
    let err = build_error(&entry);
    assert!(err.contains("expected `Buffer['T 4]`"), "{err}");
    assert!(err.contains("found `Buffer['T 8]`"), "{err}");
}

/// R7/R8a: calling `capacity` with a matching `array[u8 256]`/`Buffer[u8
/// 256]` pair typechecks and returns `256` -- the positive signature-
/// unification case, proving `'N` unifies across the two occurrences.
#[test]
fn word_over_buffer_length_unifies_against_concrete_caller() {
    let src = format!(
        "{BUFFER_HEADER}\
         : mk_buffer ( array[u8 256] -- Buffer[u8 256] ) Buffer ;\n\
         {CAPACITY_WORD}\
         : main ( -- )
           0 >u8 256 fill |arr|
           0 >u8 256 fill mk_buffer |buf|
           arr &buf capacity .
           buf drop
         ;\n"
    );
    let (_t, entry) = single_file("unify", &src);
    let out = build_and_run(&entry);
    assert_eq!(out, "256\n");
}

/// The mutation-5 discriminator (round 4): calling `capacity` with an
/// `array[u8 256]` first operand and a `Buffer[u8 512]` second operand (the
/// *same* declared `'N`, mismatched concrete lengths) is rejected --
/// `unify_poly_input`'s `Generic` arm must bind `len_args` and conflict
/// against the `Array` operand's own prior binding, not skip it.
#[test]
fn word_over_buffer_length_rejects_a_mismatched_length_operand() {
    let src = format!(
        "{BUFFER_HEADER}\
         : mk_buffer512 ( array[u8 512] -- Buffer[u8 512] ) Buffer ;\n\
         {CAPACITY_WORD}\
         : main ( -- )
           0 >u8 256 fill |arr|
           0 >u8 512 fill mk_buffer512 |buf|
           arr &buf capacity .
           buf drop
         ;\n"
    );
    let (_t, entry) = single_file("conflict", &src);
    let err = build_error(&entry);
    assert!(
        err.contains("resolved length `'N`"),
        "expected a length-conflict diagnostic naming 'N: {err}"
    );
    assert!(err.contains("256"), "{err}");
    assert!(err.contains("512"), "{err}");
}

/// R8b's own exit golden: `impl: Show for Buffer['T 4]` and `impl: Show for
/// Buffer['T 8]` do not overlap, and a bound, trait-generic call through
/// each (`shows ['T: Show] ( &'T -- ) show`, resolved through `impl:`-
/// target dispatch, not a direct concretely-typed call) prints a
/// distinguishable, hardcoded per-impl constant -- proving dispatch
/// actually reached the matching impl. Must fail (either a spurious
/// overlap-conflict diagnostic, the wrong impl's constant printing, or
/// both printing the same constant) if `match_impl_target_rec`/the
/// `collect_*` specificity family stay length-blind.
#[test]
fn impl_target_over_distinct_buffer_lengths_does_not_overlap() {
    let src = format!(
        "{BUFFER_HEADER}\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for Buffer['T 4]
           : show | b | 1 . b drop ;
         ;\n\
         impl: Show for Buffer['T 8]
           : show | b | 2 . b drop ;
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : mk4 ( array[u8 4] -- Buffer[u8 4] ) Buffer ;\n\
         : mk8 ( array[u8 8] -- Buffer[u8 8] ) Buffer ;\n\
         : main ( -- )
           0 >u8 4 fill mk4 |b4|
           0 >u8 8 fill mk8 |b8|
           &b4 shows
           &b8 shows
           b4 drop
           b8 drop
         ;\n"
    );
    let (_t, entry) = single_file("overlap", &src);
    let out = build_and_run(&entry);
    assert_eq!(out, "1\n2\n");
}
