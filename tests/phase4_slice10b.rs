//! Phase 4 Slice 10b, phase 1 goldens.
//!
//! Two independent checker changes, both prerequisites for moving `times` out
//! of the compiler and into `lib/combinators.sth`:
//!
//! * **P0**: `check_linear_across_back_edge` takes a frame floor, passed only
//!   at a spliced self-tail combinator's site. A linear local bound below the
//!   floor (the enclosing word's own, parked across the loop) stops being
//!   reported as live across the back-edge. It is still disposed: end-of-scope
//!   disposal and the branch-join guard are what enforce that, and the reject
//!   goldens below pin them.
//! * **R10**: the `drop`-visibility gate reads the module a term was *written*
//!   in (`span.module`), not the module it is being checked in
//!   (`ctx.module()`), which under splicing is the library the body was
//!   substituted into.
//!
//! Positive goldens assert full stdout and the exit code. Negative goldens
//! assert message *wording*, never a line number (a library-spliced rejection's
//! span points into `lib/combinators.sth`) and never a bare local name (a
//! spliced local is reported mangled, e.g. `leak__inl0`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// Compile and run `src`, returning stdout and the exit code.
fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = driver::build(&path).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

/// The diagnostic for a program that `import:`s the real combinator library:
/// resolving the import needs the full driver.
fn build_check_error(name: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let err = driver::build(&path).expect_err("build should fail its check");
    std::fs::remove_file(&path).ok();
    err
}

/// An `import:` line for the committed combinator library by *absolute* path,
/// so a temp source built under `temp_dir()` resolves it regardless of cwd.
fn combinators_import(qualifier: &str) -> String {
    format!(
        "import: {qualifier} \"{}/lib/combinators.sth\" ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The linear stand-in: a one-field struct with a `drop` overload that prints
/// its tag, so disposal is observable in stdout.
const SPY_DEF: &str = "type: Spy tag i64 ;\n\
    : drop ( Spy -- )  | s | s Spy>tag . ;\n";

/// The `times-helper` shape 10b's library `times` will be built on: an ordinary
/// self-tail combinator carrying a from/to pair over a row.
const TIMES_HELPER: &str = ": times-helper inline ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
    \x20 | f | | to | | from |\n\
    \x20 from to < [\n\
    \x20   from f call\n\
    \x20   from 1 + to f times-helper\n\
    \x20 ] [\n\
    \x20 ] if ;\n";

#[test]
fn times_helper_hand_copy_is_pinned_to_the_library() {
    common::assert_pinned_to_combinators_lib(TIMES_HELPER, &[]);
}

/// A scratch closure of source files, removed on drop.
struct Closure(PathBuf);

impl Closure {
    fn new(tag: &str) -> Closure {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-10b-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Closure(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Closure {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// -- P0 accept: a parked linear local crosses a spliced self-tail combinator --

#[test]
fn parked_linear_local_crosses_a_library_style_self_tail_combinator() {
    // The shape `examples/inplace_fold.sth`'s `prefix-linear` needs and only
    // the `times` intrinsic used to permit: `sp` is bound before the loop,
    // never mentioned inside it, and disposed after it. It is below the floor
    // the splice site passes, so the second clause of
    // `check_linear_across_back_edge` no longer fires on it. The `Spy` drop
    // prints `7`, so the golden shows the disposal actually ran, not merely
    // that the program compiled.
    let src = format!(
        "{SPY_DEF}{TIMES_HELPER}\
        : main ( -- )\n\
        \x20 7 Spy | sp |\n\
        \x20 0 0 4 [ | i | i + ] times-helper .\n\
        \x20 sp drop ;\n"
    );
    let (stdout, code) = run_src("10b_parked_helper", &src);
    assert_eq!(stdout, "6\n7\n");
    assert_eq!(code, 0);
}

#[test]
fn parked_linear_local_crosses_while() {
    // The same shape over the library's `while`: P0 relaxes every spliced
    // self-tail combinator, not just the one 10b is about to add, so `while`
    // gets its own accept rather than riding on the helper's.
    let src = format!(
        "{SPY_DEF}{}: main ( -- )\n\
         \x20 7 Spy | sp |\n\
         \x20 0 [ dup 5 < [ 1 + true ] [ false ] if ] c::while .\n\
         \x20 sp drop ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("10b_parked_while", &src);
    assert_eq!(stdout, "5\n7\n");
    assert_eq!(code, 0);
}

// -- P0's tripwire: the shape the floor exempts but must stay rejected --------

#[test]
fn own_frame_linear_bound_before_the_tail_if_is_error() {
    // The floor is an `if`-arm entry depth, not a frame lifetime, so it also
    // exempts a linear bound in the combinator's *own* frame before the tail
    // `if` -- a local that is re-created every iteration and has no
    // ancestor-frame lifetime at all. That over-exemption is harmless only
    // because end-of-scope disposal independently rejects it. This golden is
    // the tripwire for that co-guard: if end-of-scope disposal is ever widened
    // or removed, the shape starts compiling and this goes red.
    let src = format!(
        "{SPY_DEF}\
         : while inline ( i64 [ i64 -- i64 bool ] -- i64 )\n\
         \x20 | p | 9 Spy | own | p call [ p while ] [ ] if ;\n\
         : main ( -- ) 0 [ dup 5 < [ 1 + true ] [ false ] if ] while . ;\n"
    );
    let err = build_check_error("10b_own_frame_linear", &src);
    assert!(
        err.contains("is never consumed")
            && err.contains("`Spy`, which is linear")
            && err.contains("nothing is dropped for you"),
        "an own-frame linear bound before the tail `if` gets the scope-end rejection, got: {err}"
    );
}

// -- the four pre-existing guards P0 leans on --------------------------------
//
// These are not P0's own code. They are pinned because P0's argument is that
// they, not the relaxed clause, are what makes an unconsumed linear an error;
// a golden each keeps that claim falsifiable. Three of the four are indifferent
// to P0; the fourth (guard 4) is the accept golden minus its disposal, so
// neutering the floor moves its rejection from scope end to the back edge and
// flips it.

#[test]
fn quotation_consuming_an_enclosing_linear_is_rejected_by_capture_admission() {
    // Guard 1 (D3 capture admission): consuming the parked local *inside* the
    // loop body would dispose it once per iteration. Rejected at the literal,
    // before the back-edge is ever reached. The consume sits outside the body's
    // own `if` on purpose, so the local reaches the literal's exit `Moved`
    // rather than `MaybeMoved`: that is what keeps this golden distinct from
    // the branch-join one below, which is the only one of the two the
    // `MaybeMoved` arm carries.
    let src = format!(
        "{SPY_DEF}{}: main ( -- )\n\
         \x20 7 Spy | sp |\n\
         \x20 0 [ sp drop dup 5 < [ 1 + true ] [ false ] if ] c::while . ;\n",
        combinators_import("c")
    );
    let err = build_check_error("10b_hazard_consume", &src);
    assert!(
        err.contains(
            "the quotation passed to `while` consumes the enclosing local `sp`, which is linear"
        ),
        "consuming an enclosing linear in a loop body is a capture-admission rejection, got: {err}"
    );
}

#[test]
fn quotation_consuming_an_enclosing_linear_on_one_branch_is_rejected() {
    // Guard 2 (the branch-join `MaybeMoved` state, sharing capture
    // admission's match arm): consumed on one arm only, so the value's state
    // across the edge is neither live nor moved. The assertion is on the D3
    // wording deliberately: removing `MaybeMoved(_)` from that arm still
    // rejects the program, with `use after move`, so an `is_err()` golden here
    // would pass under the mutation and pin nothing.
    let src = format!(
        "{SPY_DEF}{}: main ( -- )\n\
         \x20 7 Spy | sp |\n\
         \x20 0 [ dup 5 < [ dup 2 > [ sp drop ] [ ] if 1 + true ] [ false ] if ] c::while . ;\n",
        combinators_import("c")
    );
    let err = build_check_error("10b_hazard_branch", &src);
    assert!(
        err.contains(
            "the quotation passed to `while` consumes the enclosing local `sp`, which is linear"
        ),
        "a one-branch consume takes the same capture-admission rejection, got: {err}"
    );
}

#[test]
fn linear_bound_inside_a_loop_body_and_left_unconsumed_is_rejected() {
    // Guard 3 (end-of-scope disposal): a linear bound *inside* the quotation
    // body. Its scope closes at the body's own end, before the back-edge check
    // runs at all, so this rejection is independent of P0 either way.
    let src = format!(
        "{SPY_DEF}{}: main ( -- )\n\
         \x20 0 [ dup 5 < [ 5 Spy | tmp | 1 + true ] [ false ] if ] c::while . ;\n",
        combinators_import("c")
    );
    let err = build_check_error("10b_hazard_body_local", &src);
    assert!(
        err.contains("is never consumed") && err.contains("`Spy`, which is linear"),
        "a linear bound inside the body is a scope-end rejection, got: {err}"
    );
}

#[test]
fn parked_linear_local_never_disposed_at_all_is_rejected() {
    // Guard 4 (end-of-scope disposal, at the enclosing word): the accept
    // golden above with its trailing `sp drop` removed. This is what makes the
    // accept an admission of a *deferred* obligation rather than of a leak.
    //
    // Unlike guards 1-3 this one is P0-sensitive, because it is the accept
    // shape: neuter the floor and the back-edge clause rejects the same
    // program one step earlier, with its own wording. Asserting the scope-end
    // wording is deliberate, since that is what the guard being pinned here
    // actually says.
    let src = format!(
        "{SPY_DEF}{}: main ( -- )\n\
         \x20 7 Spy | sp |\n\
         \x20 0 [ dup 5 < [ 1 + true ] [ false ] if ] c::while . ;\n",
        combinators_import("c")
    );
    let err = build_check_error("10b_hazard_never_disposed", &src);
    assert!(
        err.contains("is never consumed") && err.contains("drop it or return it"),
        "a parked linear that is never disposed is still rejected at scope end, got: {err}"
    );
}

// -- R10: the disposal-visibility gate follows the authoring module ----------

#[test]
fn spliced_body_disposes_a_locally_declared_linear() {
    // The gap R10 closes: the quotation is written in `main`'s module, which
    // declares `Spy` and its `drop`, but it is checked spliced into
    // `lib/combinators.sth`, which has never heard of `Spy`. Gating on
    // `ctx.module()` rejected this with the fabricated transitive-import note.
    // Each iteration builds and disposes a `Spy`, so the three destructor runs
    // are visible in stdout before the final counter.
    let src = format!(
        "{SPY_DEF}{}: main ( -- )\n\
         \x20 0 [ dup 3 < [ dup Spy drop 1 + true ] [ false ] if ] c::while . ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("10b_r10_accept", &src);
    assert_eq!(stdout, "0\n1\n2\n3\n");
    assert_eq!(code, 0);
}

#[test]
fn spliced_body_disposing_a_qualified_only_imported_type_is_error() {
    // A rule-intactness pin, not a splice witness: this program's `drop` is
    // caught by the home-scope pass before the quotation is ever spliced, so
    // it exercises the same gate as `imported_linear_type_dropped_without_
    // importing_it_is_error` and only goes red if the gate is deleted
    // outright. Kept anyway as coverage that R10's `span.module` change does
    // not loosen the qualified-only-import rule end to end.
    let c = Closure::new("r10-reject");
    c.write(
        "lib.sth",
        "export: Res make run ;\n\
         type: Res n i64 ;\n\
         : drop ( Res -- )  | r | r Res>n . ;\n\
         : make ( i64 -- Res ) Res ;\n\
         : run ( Res [ Res -- ] -- )  | f | f call ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: lib \"lib.sth\" ;\n\
         : main ( -- ) 5 lib::make [ drop ] lib::run ;\n",
    );
    let err = driver::build(&entry).expect_err("build should fail its check");
    assert!(
        err.contains("cannot `drop` a value of type `lib::Res` in `main`")
            && err.contains("has not imported by name"),
        "a qualified-only imported type stays undisposable from the importing module, got: {err}"
    );
}

#[test]
fn drop_visibility_error_is_worded_from_the_authoring_module_under_nested_splicing() {
    // A quotation literal is re-validated against each nesting level's own
    // live row (`check_poly_combinator_args`, row = `stack[..base]`), so an
    // inner combinator can push its own type into the row before the
    // caller's literal is re-checked under that inner combinator's `ctx`.
    // Here `main`'s quotation `[ drop 0 ]` is re-validated inside `c::inner`'s
    // splice, where `ctx.module()` is `c`, but `drop`'s span still names
    // `main`, the module it was written in. `Res` is declared in `b`, which
    // `main` imports only as a bare `import: b "b.sth"`, not by name, so the
    // drop is rejected -- but which module's import map supplies the
    // qualifier depends on which of `span.module` (`main`, which does import
    // `b`) or `ctx.module()` (`c`, which does not) is used: the two give a
    // different qualifier and a different remedy. This is the accept-side
    // twin of `spliced_body_disposing_a_qualified_only_imported_type_is_error`
    // and, unlike that one, only goes red under R9 mutation (d) (revert the
    // gate's `caller` derivation to `ctx.module()`).
    let c = Closure::new("nested-splice-divergence");
    c.write(
        "c.sth",
        "export: inner ;\n\
         : inner inline ( ..s ~[ ..s -- ..s ] -- ..s )  | f | f call ;\n",
    );
    c.write(
        "b.sth",
        "import: c \"c.sth\" ;\n\
         export: outer Res ;\n\
         type: Res n i64 ;\n\
         : drop ( Res -- )  | r | r Res>n . ;\n\
         : outer inline ( ..s ~[ ..s -- ..s ] -- ..s )  | f | 9 Res f c::inner ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: b \"b.sth\" ;\n\
         : main ( -- ) 1 [ drop 0 ] b::outer . ;\n",
    );
    let err = driver::build(&entry).expect_err("build should fail its check");
    assert!(
        err.contains("cannot `drop` a value of type `b::Res` in `main`")
            && err.contains("has not imported by name"),
        "the span-authoring module (`main`, which imports `b`) supplies the \
         qualifier and remedy, not the splice-time `ctx` (`c`, which does \
         not import `b`), got: {err}"
    );
}

// -- R7: exit witnesses on the real, library `times` -------------------------
//
// Each asserts full stdout plus the exit code. `times` is now ordinary Sooth
// source, so these are the only tests that pin what it *computes*; the goldens
// the intrinsic had asserted the compiler's own arm.

/// Build `src` and return the binary's path, so a golden can run it under a
/// reduced `ulimit -s` (the constant-stack witnesses) rather than plainly.
fn build_binary(name: &str, src: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = driver::build(&path).expect("build should succeed");
    std::fs::remove_file(&path).ok();
    binary
}

/// Run `binary` under `ulimit -s {limit_kb}`, returning the exit code (`None`
/// if it died by signal, e.g. a `SIGSEGV` from an overflowed stack) and stdout.
fn run_at_stack_limit(binary: &std::path::Path, limit_kb: u32) -> (Option<i32>, String) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -s {limit_kb} && exec \"{}\"",
            binary.display()
        ))
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    )
}

#[test]
fn times_sums_the_index_over_five_iterations() {
    // The headline value: `[ + ]` adds each 0-based index into the row seed,
    // so 0+1+2+3+4 = 10 -- the library `times` hands the body the same index
    // sequence the intrinsic did.
    let src = format!(
        "{}: main ( -- ) 0 5 [ + ] times . ;\n",
        combinators_import("c | times |")
    );
    let (stdout, code) = run_src("10b_times_sum", &src);
    assert_eq!(stdout, "10\n");
    assert_eq!(code, 0);
}

#[test]
fn times_runs_one_million_iterations_in_constant_stack() {
    // The constant-stack guarantee, which is what makes `times-helper`'s
    // self-tail call a loop back-edge rather than a million splices. The
    // *printed value* is asserted, not just the exit code: a loop that runs
    // zero iterations also exits 0.
    let src = format!(
        "{}: main ( -- ) 0 1000000 [ drop 1 + ] times . ;\n",
        combinators_import("c | times |")
    );
    let binary = build_binary("10b_times_1m", &src);
    let (code, out) = run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!((code, out.as_str()), (Some(0), "1000000"));
}

#[test]
fn times_carries_an_aggregate_through_the_row() {
    // A `map`-shaped body: the array rides the *row* through every iteration --
    // the body pops it into a frame local, borrows it mutably, writes the
    // doubled element back in place and returns it to the row -- rather than
    // staying a caller frame local the body borrows. This is the
    // aliasing-sensitive shape (a loop-carried aggregate staged across the
    // back-edge), and the printed elements pin the values.
    let src = format!(
        "{}: main ( -- )\n\
         \x20 0 4 fill\n\
         \x20 4 [ | i | | a | &!a i >usize &!> i 2 * ! a ] times\n\
         \x20 4 [ | i | | a | &a i >usize &> @ . a ] times\n\
         \x20 drop ;\n",
        combinators_import("c | times |")
    );
    let (stdout, code) = run_src("10b_times_aggregate", &src);
    assert_eq!(stdout, "0\n2\n4\n6\n");
    assert_eq!(code, 0);
}

#[test]
fn times_nested_inside_each_map_fold_and_filter_runs() {
    // The novel risk beyond 10a: a leaf combinator's call site is now three
    // splices deep (leaf -> `times` -> `times-helper`), and a `times` written
    // *inside* the leaf's own quotation argument doubles that again. All four
    // leaves are exercised in one program, each with a `times` in its body.
    //
    // `each` prints twice per element (1 1 2 2), `map` adds the inner loop's
    // 0+1 to each element (2 3), `fold` adds each element twice (2*(1+2) = 6),
    // and `filter` keeps both elements after running an inner loop per element
    // (2).
    let src = format!(
        "{}: pair ( -- [i64 2] )\n\
         \x20 0 2 fill | s |\n\
         \x20 &!s 0 >usize &!> 1 !\n\
         \x20 &!s 1 >usize &!> 2 !\n\
         \x20 s ;\n\
         : main ( -- )\n\
         \x20 pair [ | v | 2 [ drop v . ] c::times ] c::each\n\
         \x20 pair [ 2 [ | i | i + ] c::times ] c::map [ . ] c::each\n\
         \x20 pair 0 [ | acc v | acc 2 [ drop v + ] c::times ] c::fold .\n\
         \x20 pair [ | v | 0 2 [ drop 1 + ] c::times drop v 0 > ] c::filter . drop ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("10b_times_in_leaves", &src);
    assert_eq!(stdout, "1\n1\n2\n2\n2\n3\n6\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn times_nested_inside_an_outer_times_runs() {
    // The other doubled-depth shape: `times` inside `times`, and a `times`
    // inside a `while` body. The outer `times` counts 3 rounds of an inner
    // 2-iteration loop (3*2 = 6), and the `while` runs its own inner `times`
    // each step while counting to 5.
    let src = format!(
        "{}: main ( -- )\n\
         \x20 0 3 [ | i | 2 [ | j | 1 + ] c::times ] c::times .\n\
         \x20 0 [ 2 [ | i | ] c::times dup 5 < [ 1 + true ] [ false ] if ] c::while . ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("10b_times_in_times", &src);
    assert_eq!(stdout, "6\n5\n");
    assert_eq!(code, 0);
}
