//! Phase 4 slice 6f, phase 3 goldens: the named-accumulator in-place fold
//! dogfood (`examples/inplace_fold.sth`). Ending a bound borrow at its last use
//! lets an aggregate accumulator stay a named local instead of loop-carried
//! state, so its `times` body writes in place with no per-iteration aggregate
//! `blit`. Each golden builds and runs the committed example (value side) and
//! then re-lowers its source to assert the measurable claim against emitted QBE
//! (structure side), since a runtime golden cannot tell "mutated in place" from
//! "rebuilt correctly".

use std::process::Command;

mod common;

const DOGFOOD: &str = "examples/inplace_fold.sth";

/// Build and run the committed dogfood, returning stdout and the exit code.
fn run_dogfood() -> (String, i32) {
    let binary = common::build_example("examples/inplace_fold.sth");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output.status.code().expect("process should exit normally"),
    )
}

/// The emitted QBE body of `word`, mangled name and all. `qbe_name` escapes each
/// non-alphanumeric character as `.{hex}.` to stay injective, so `-` (0x2d) makes
/// the fold words emit as `prefix.2d.copy`/`prefix.2d.linear`, and `resolve`
/// appends the module component, so the entry file's words emit as
/// `prefix.2d.copy__m0`. Panics if the word is absent, so a rename that silences
/// the assertion fails loudly. Every
/// function definition's whole line starts with `export function ` (see
/// `src/backend/qbe.rs`'s `"export function {ret_ty}${}("`; `ret_ty` sits
/// between `function` and the `$name`, so a bare `function `-prefix check on
/// the header itself would miss it), so this anchors on the enclosing line
/// rather than the header alone, picking the definition even if a call site
/// (`${word}(` with no `export function ` on its own line) appears earlier
/// in the emitted module.
///
/// This routes through the driver rather than calling `lex`/`parse`/`check` on
/// the source text: since 10b the dogfood `import:`s `times` from
/// `lib/combinators.sth`, and only the driver resolves an import closure.
fn fold_body(word: &str) -> String {
    let il = sooth::driver::emit_ssa(std::path::Path::new(DOGFOOD)).expect("dogfood should emit");
    let header = format!("${word}__m0(");
    let def_at = il
        .match_indices(&header)
        .map(|(i, _)| i)
        .find(|&i| {
            let line_start = il[..i].rfind('\n').map_or(0, |n| n + 1);
            il[line_start..i].starts_with("export function ")
        })
        .unwrap_or_else(|| panic!("no function definition for `{header}` in emitted IL:\n{il}"));
    let rest = &il[def_at..];
    let end = rest.find("\n}").expect("a function body ends in `}`");
    rest[..end].to_string()
}

/// A fold body that is a genuine loop mutating in place, so a "no blit"
/// assertion over it is not vacuously true of straight-line code: it has a
/// back-edge (`jmp @blk1`, the loop header its `phi`s live in) and an in-place
/// element store (`storel`).
fn assert_in_place_loop(body: &str) {
    assert!(
        body.contains("phi") && body.matches("jmp @blk1").count() >= 2,
        "the fold should lower to a loop with a back-edge: {body}"
    );
    assert!(
        body.contains("storel "),
        "the fold should write each element in place: {body}"
    );
}

#[test]
fn inplace_fold_copy_lowers_without_per_iteration_blit() {
    // T12: the dogfood at a `Copy` accumulator (`AccC`). Builds and prints the
    // prefix sums of 1 2 3 4 (1 3 6 10), and its `times` body carries no
    // aggregate `blit` because the accumulator is a named local mutated through
    // a borrow, never loop-carried state.
    let (stdout, code) = run_dogfood();
    assert_eq!(stdout, "1\n3\n6\n10\n1\n3\n6\n10\n");
    assert_eq!(code, 0);

    let body = fold_body("prefix.2d.copy");
    assert_in_place_loop(&body);
    assert!(
        !body.contains("blit"),
        "the Copy in-place fold must not copy the accumulator per iteration: {body}"
    );
}

#[test]
fn inplace_fold_linear_lowers_without_per_iteration_blit() {
    // T13: the same shape at a linear accumulator (`AccL`, made linear by its
    // `drop` overload). The borrow half carries it exactly as it carries the
    // Copy case; no aliasing rule fires for the linear one. Same no-blit claim.
    let (stdout, code) = run_dogfood();
    assert_eq!(stdout, "1\n3\n6\n10\n1\n3\n6\n10\n");
    assert_eq!(code, 0);

    let body = fold_body("prefix.2d.linear");
    assert_in_place_loop(&body);
    assert!(
        !body.contains("blit"),
        "the linear in-place fold must not copy the accumulator per iteration: {body}"
    );
}
