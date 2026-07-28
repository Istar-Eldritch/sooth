//! Goldens for reference types, places, projection and access.
//!
//! Kept out of `tests/phase0.rs` deliberately: that file is asserted never to
//! change from this work's base commit, so a new golden belongs somewhere the
//! addition-only check has nothing to reason about.

use std::io::Write;
use std::process::{Command, Stdio};

use sooth::{check, lexer, parser};

/// Compile and run `src`, returning its stdout and exit code. `name`
/// distinguishes the temp source (and so the emitted binary) per test, since
/// the goldens run in parallel in one process.
fn run_src(name: &str, src: &str) -> (String, i32) {
    let (stdout, _stderr, code) = run_src_traced(name, src, false);
    (stdout, code)
}

/// `run_src` with stderr too, and the allocation trace optionally enabled: the
/// trace shares stdout with the program's own output, so the caller reads one
/// transcript in program order.
fn run_src_traced(name: &str, src: &str, trace: bool) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    let mut cmd = Command::new(&binary);
    match trace {
        true => cmd.env(sooth::ir::TRACE_ALLOC_ENV, "1"),
        false => cmd.env_remove(sooth::ir::TRACE_ALLOC_ENV),
    };
    let output = cmd.output().expect("binary should run");
    std::fs::remove_file(&path).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        String::from_utf8(output.stderr).expect("stderr should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

fn parse_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    parser::parse(&tokens).expect_err("parsing should fail")
}

/// Run a scripted REPL session (one input line per element of `lines`) and
/// return the whole captured stdout, mirroring `tests/phase1.rs`'s helper.
fn run_session(lines: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("repl")
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("repl should spawn");
    let mut stdin = child.stdin.take().expect("stdin should be piped");
    let script = lines.join("\n") + "\n";
    stdin
        .write_all(script.as_bytes())
        .expect("writing stdin should succeed");
    drop(stdin);
    let output = child.wait_with_output().expect("repl should exit cleanly");
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

/// The buffer half of the slice's dogfood: `push-byte` mutates through a
/// `&!Buf` with no rebuild, `byte-at` reads through a `&Buf`, and `copy-byte`
/// holds one of each, rooted at two different places.
const BUFFER_DOGFOOD: &str = "\
type: Buf  data ^[u8 64]  len usize ;

: new ( -- Buf )
  0 >u8 64 fill ^ 0 >usize Buf ;

: push-byte ( &!Buf u8 -- )
  | b x |
  b &!Buf>len @ | i |
  b &!Buf>data &!^ | arr |
  arr i &!> x !
  b &!Buf>len 1 +! ;

: byte-at ( &Buf usize -- u8 )
  | b i |
  b &Buf>data &^ i &> @ ;

: copy-byte ( &!Buf &Buf usize -- )
  | dst src i |
  dst src i byte-at push-byte ;
";

// --- `&`/`&!` are prefix borrows of a place, and only of a place

#[test]
fn borrow_of_place_is_accepted() {
    let (stdout, code) = run_src(
        "borrow-of-place",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  &v &V>x @ .\n  &!v &!V>y @ . ;\n",
    );
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn borrow_of_literal_is_error() {
    let err = check_error(": main ( -- )\n  &5 drop ;\n");
    assert!(
        err.contains("`&5` does not borrow a place"),
        "expected the non-place diagnostic: {err}"
    );
    assert!(
        err.contains("is a literal, not a local"),
        "the error should name what was found: {err}"
    );
    assert!(err.contains("line 2"), "the error should locate it: {err}");
}

#[test]
fn borrow_of_arithmetic_result_is_error() {
    // Prefix, so the only way to aim a borrow at a computed value is a bare
    // sigil after it. A place is a local name, never "whatever is on top".
    let err = check_error(": main ( -- )\n  1 2 + & drop ;\n");
    assert!(
        err.contains("`&` does not borrow a place"),
        "expected the non-place diagnostic: {err}"
    );
    assert!(
        err.contains("bind the value with `| name |` first"),
        "the error should point at the remedy: {err}"
    );
}

#[test]
fn borrow_of_word_result_is_error() {
    let err = check_error(": five ( -- i64 ) 5 ;\n: main ( -- )\n  &five drop ;\n");
    assert!(
        err.contains("`&five` does not borrow a place"),
        "expected the non-place diagnostic: {err}"
    );
    assert!(
        err.contains("`five` is not a local in scope"),
        "the error should name what was found: {err}"
    );
}

#[test]
fn borrow_led_name_is_reserved() {
    for (src, kind) in [
        (": &grab ( -- ) ;\n", "word"),
        ("type: &Thing x i64 ;\n", "type"),
        (": main ( i64 -- ) | &a | ;\n", "local"),
    ] {
        let err = parse_error(src);
        assert!(
            err.contains("reserved for the reference syntax"),
            "expected the reservation error for a {kind} name: {err}"
        );
    }
}

#[test]
fn shadowing_builtin_access_word_is_error() {
    for name in ["@", "!", "+!"] {
        let err = parse_error(&format!(": {name} ( i64 -- ) . ;\n"));
        assert!(
            err.contains("is a builtin access word"),
            "expected the shadowing rejection for `{name}`: {err}"
        );
        assert!(err.contains("line 1"), "the error should locate it: {err}");
    }
}

// --- only an aggregate local is a borrow root; `&T` is Copy

#[test]
fn borrow_of_scalar_local_is_error() {
    let err = check_error(": main ( -- )\n  5 | n |\n  &n drop ;\n");
    assert!(
        err.contains("cannot borrow the scalar local `n` of type `i64`"),
        "expected the scalar-root rejection: {err}"
    );
    assert!(
        err.contains("borrow a field or an aggregate"),
        "the error should point at the remedy: {err}"
    );
    assert!(err.contains("line 3"), "the error should locate it: {err}");
}

#[test]
fn borrow_of_moved_local_is_error() {
    // A borrow is not a move, but the referent still has to exist: `b` was
    // consumed (and its cell freed) before the borrow, so projecting through
    // the reference would read freed storage.
    let err = check_error(
        "type: Box c ^i64 ;\n\
         : main ( -- )\n  7 ^ Box | b |\n  b drop\n  &b &Box>c &^ @ . ;\n",
    );
    assert!(
        err.contains("use after move") && err.contains("local `b` of type `Box`"),
        "borrowing a consumed local must name the move that consumed it: {err}"
    );
    assert!(err.contains("line 4"), "the error should locate it: {err}");
}

#[test]
fn borrow_of_reference_local_is_error() {
    // A reference parameter needs no sigil, so the sigil a reader might
    // add by habit gets its own diagnostic rather than the scalar-root one.
    let err = check_error("type: V x i64 y i64 ;\n: f ( &!V -- ) | b |\n  &!b &!V>x 1 +! ;\n");
    assert!(
        err.contains("it is already the reference `&!V`"),
        "expected the already-a-reference rejection: {err}"
    );
    assert!(
        err.contains("write `b`, not `&!b`"),
        "the error should point at the remedy: {err}"
    );
}

#[test]
fn projection_to_scalar_field_is_accepted() {
    // What is rejected is a scalar *root*, not a scalar *result*: the referent here is
    // a field inside an aggregate that already has a slot.
    let (stdout, code) = run_src(
        "projection-to-scalar-field",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  &!v &!V>x 41 +!\n  &v &V>x @ . ;\n",
    );
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

#[test]
fn dup_of_shared_reference_is_accepted() {
    let (stdout, code) = run_src(
        "dup-of-shared-reference",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  7 8 V | v |\n  &v dup &V>x @ . &V>y @ . ;\n",
    );
    assert_eq!(stdout, "7\n8\n");
    assert_eq!(code, 0);
}

#[test]
fn dup_of_mutable_reference_is_error() {
    // Two live `&!` to one place, rejected by `dup`'s existing Copy gate. The
    // explanation must cite exclusivity, not ownership: a reference owns
    // nothing, so calling it linear would contradict the type rule.
    let err = check_error("type: V x i64 y i64 ;\n: main ( -- )\n  1 2 V | v |\n  &!v dup ;\n");
    assert!(
        err.contains("`dup`") && err.contains("&!V") && err.contains("is exclusive"),
        "expected `dup` to reject a mutable reference as exclusive: {err}"
    );
    assert!(
        !err.contains("is linear"),
        "a reference owns nothing, so the message must not call it linear: {err}"
    );
}

// --- projection through all three shapes, one spelling per
//     mutability

#[test]
fn projection_through_field_element_and_cell_reads_correctly() {
    let (stdout, code) = run_src(
        "projection-three-shapes",
        &format!(
            "{BUFFER_DOGFOOD}\n\
             : main ( -- )\n\
             \x20 new | a |\n\
             \x20 &!a 72 >u8 push-byte\n\
             \x20 &!a 90 >u8 push-byte\n\
             \x20 &a 0 byte-at .\n\
             \x20 &a 1 byte-at .\n\
             \x20 &a &Buf>len @ .\n\
             \x20 a drop ;\n"
        ),
    );
    assert_eq!(stdout, "72\n90\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn element_projection_out_of_bounds_still_traps() {
    // `&>`/`&!>` reuse `get`'s runtime guard: a computed out-of-range index
    // traps rather than reading past the array.
    let (stdout, stderr, code) = run_src_traced(
        "element-projection-oob",
        ": at ( &[u8 4] usize -- u8 ) &> @ ;\n\
         : main ( -- )\n  0 >u8 4 fill | arr |\n  1 .\n  &arr 4 1 + >usize at .\n  99 . ;\n",
        false,
    );
    assert_eq!(stdout, "1\n", "the sentinel before the trap should print");
    assert_ne!(code, 0, "an out-of-bounds projection must exit nonzero");
    assert!(
        stderr.contains("out of range"),
        "the trap should say it is out of range: {stderr}"
    );
}

#[test]
fn mutable_element_projection_through_shared_reference_is_error() {
    // `&!>` requires a mutable-reference receiver; borrowing the array
    // shared with `&` and then projecting with `&!>` is a receiver-mutability
    // mismatch, not a bounds or type error.
    let err = check_error(": main ( -- ) 0 4 fill | a | &a 0 &!> 99 ! &a 0 &> @ . ;");
    assert!(
        err.contains("`&!>` expected `&![i64 4]`, found `&[i64 4]`"),
        "expected the receiver-mutability mismatch naming both types: {err}"
    );
}

#[test]
fn mutable_element_projection_through_mutable_reference_is_accepted() {
    // The converse of the above: borrowing the array mutably with `&!` and
    // projecting with the matching `&!>` is accepted.
    let (stdout, code) = run_src(
        "element-projection-mutable-receiver",
        ": main ( -- ) 0 4 fill | a | &!a 0 &!> 99 ! &a 0 &> @ . ;",
    );
    assert_eq!(stdout, "99\n");
    assert_eq!(code, 0);
}

#[test]
fn store_through_shared_reference_is_error() {
    let err = check_error(
        "type: V x i64 y i64 ;\n: main ( -- )\n  1 2 V | v |\n  &v &V>x 5 !\n  v V> . . ;\n",
    );
    assert!(
        err.contains("`!` cannot store through the shared reference `&i64`"),
        "expected the shared-store rejection: {err}"
    );
    assert!(
        err.contains("borrow it mutably with `&!`"),
        "the error should point at the remedy: {err}"
    );
}

// --- `@`, `!`, `+!`

#[test]
fn access_through_reference_reads_and_writes() {
    let (stdout, code) = run_src(
        "access-reads-and-writes",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  &!v &!V>x 40 !\n  &v &V>x @ .\n  &v &V>y @ . ;\n",
    );
    assert_eq!(stdout, "40\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn increment_through_mutable_reference_adds_in_place() {
    let (stdout, code) = run_src(
        "increment-in-place",
        "type: Counter n usize ;\n\
         : main ( -- )\n  0 >usize Counter | c |\n  &!c &!Counter>n 1 +!\n  \
         &!c &!Counter>n 1 +!\n  &c &Counter>n @ . ;\n",
    );
    assert_eq!(stdout, "2\n");
    assert_eq!(code, 0);
}

#[test]
fn fetch_of_linear_referent_is_error() {
    let err =
        check_error("type: Holds a __spy ;\n: peek ( &Holds -- ) | h |\n  h &Holds>a @ drop ;\n");
    assert!(
        err.contains("`@` cannot access the linear referent `__spy`"),
        "expected the linear-fetch rejection: {err}"
    );
    assert!(
        err.contains("second owner"),
        "the error should say why: {err}"
    );
}

#[test]
fn store_of_linear_referent_is_error() {
    let err = check_error(
        "type: Holds a __spy ;\n: put ( &!Holds __spy -- ) | h s |\n  h &!Holds>a s ! ;\n",
    );
    assert!(
        err.contains("`!` cannot access the linear referent `__spy`"),
        "expected the linear-store rejection: {err}"
    );
    assert!(
        err.contains("silently leak"),
        "the error should say why: {err}"
    );
}

#[test]
fn fetch_or_store_of_copy_aggregate_reads_and_writes() {
    // The Copy restriction is Copy-vs-linear, never scalar-vs-aggregate: a
    // Copy struct fetches via `Alloc`+`Blit` and stores via `Blit`.
    let (stdout, code) = run_src(
        "copy-aggregate-access",
        "type: V x i64 y i64 ;\n\
         type: Holder v V ;\n\
         : main ( -- )\n\
         \x20 1 2 V Holder | h |\n\
         \x20 &h &Holder>v @ | got |\n\
         \x20 got V> . .\n\
         \x20 8 9 V | fresh |\n\
         \x20 &!h &!Holder>v fresh !\n\
         \x20 &h &Holder>v @ V> . . ;\n",
    );
    assert_eq!(stdout, "2\n1\n9\n8\n");
    assert_eq!(code, 0);
}

#[test]
fn fetch_of_copy_aggregate_survives_source_mutation() {
    // A fetch that returned the field address instead of copying would read
    // and write correctly right up until the source is mutated behind it.
    let (stdout, code) = run_src(
        "copy-aggregate-fetch-independent",
        "type: V x i64 y i64 ;\n\
         type: Holder v V ;\n\
         : main ( -- )\n\
         \x20 1 2 V Holder | h |\n\
         \x20 &h &Holder>v @ | got |\n\
         \x20 &!h &!Holder>v &!V>x 99 !\n\
         \x20 got V> . .\n\
         \x20 &h &Holder>v &V>x @ . ;\n",
    );
    assert_eq!(
        stdout, "2\n1\n99\n",
        "the fetched copy must still read its pre-mutation value"
    );
    assert_eq!(code, 0);
}

// --- escape is prevented structurally

#[test]
fn reference_in_struct_field_is_error() {
    let err = check_error("type: Bad r &i64 ;\n: main ( -- ) ;\n");
    assert!(
        err.contains("a reference cannot be stored")
            && err.contains("field `r` of type `Bad`")
            && err.contains("`&i64`"),
        "expected the struct-field rejection: {err}"
    );
}

#[test]
fn reference_in_enum_payload_is_error() {
    let err = check_error("type: Bad | Empty | Full r &!i64 ;\n: main ( -- ) ;\n");
    assert!(
        err.contains("a reference cannot be stored")
            && err.contains("payload field `r` of variant `Full`"),
        "expected the enum-payload rejection: {err}"
    );
}

#[test]
fn reference_as_array_element_is_error() {
    // `fill` accepts any `Copy` element and `&T` is `Copy`, so this is caught
    // at the construction site, with no declaration anywhere to sweep.
    let err = check_error(
        "type: V x i64 y i64 ;\n: main ( -- )\n  1 2 V | v |\n  &v 4 fill drop\n  v drop ;\n",
    );
    assert!(
        err.contains("a reference cannot be stored")
            && err.contains("the element `fill` would store")
            && err.contains("`&V`"),
        "expected the array-element rejection: {err}"
    );
}

#[test]
fn reference_in_cell_payload_is_error() {
    let err = check_error(
        "type: V x i64 y i64 ;\n: main ( -- )\n  1 2 V | v |\n  &v ^ drop\n  v drop ;\n",
    );
    assert!(
        err.contains("a reference cannot be stored") && err.contains("the payload `^` would store"),
        "expected the cell-payload rejection: {err}"
    );
}

#[test]
fn reference_returned_from_word_is_error() {
    // The direct consequence: a projection can never be factored into its own
    // helper word.
    let err = check_error(
        "type: Buf data ^[u8 64] len usize ;\n: len-of ( &!Buf -- &!usize ) &!Buf>len ;\n",
    );
    assert!(
        err.contains("a reference cannot be stored")
            && err.contains("`len-of` declares the output `&!usize`"),
        "expected the effect-output rejection: {err}"
    );
}

#[test]
fn reference_surviving_repl_line_is_error() {
    // Reachable only since Slice 5 gave a REPL line locals: a line can now
    // form a place, and the session's inter-line stack outlives it.
    let out = run_session(&["type: V x i64 y i64 ;", "1 2 V | v | &v", "7 ."]);
    assert!(
        out.contains("a reference cannot be stored") && out.contains("carries into the next line"),
        "expected the carried-stack rejection: {out}"
    );
    assert!(
        out.contains("7"),
        "the session should survive the rejected line: {out}"
    );
}

#[test]
fn reference_in_effect_input_is_accepted() {
    let (stdout, code) = run_src(
        "reference-input-accepted",
        "type: V x i64 y i64 ;\n\
         : sum ( &V -- i64 ) | v |\n  v &V>x @ v &V>y @ + ;\n\
         : main ( -- )\n  3 4 V | v |\n  &v sum . ;\n",
    );
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

#[test]
fn drop_of_reference_frees_nothing() {
    // The cell inside `Box` is allocated once and freed once, by the real
    // owner's disposal; dropping a reference to the box contributes neither.
    let (stdout, _stderr, code) = run_src_traced(
        "drop-of-reference",
        "type: Box c ^i64 ;\n\
         : main ( -- )\n  7 ^ Box | b |\n  &!b drop\n  &b drop\n  b Box> ^> . ;\n",
        true,
    );
    assert_eq!(stdout, "alloc 8\nfree 8\n7\n");
    assert_eq!(code, 0);
}

// --- `&!T` lowers to an opaque pointer, not a by-value aggregate

#[test]
fn mutation_through_reference_parameter_is_visible_to_caller() {
    // QBE's C-ABI classification passes a `:Counter`-spelled parameter by
    // value, so this is exactly the golden that fails if a reference ever
    // stops mapping to `IrType::Ptr`.
    let (stdout, code) = run_src(
        "mutation-visible-to-caller",
        "type: Counter n i64 ;\n\
         : bump ( &!Counter -- ) &!Counter>n 1 +! ;\n\
         : main ( -- )\n  0 Counter | c |\n  &!c bump\n  &!c bump\n  &c &Counter>n @ . ;\n",
    );
    assert_eq!(stdout, "2\n");
    assert_eq!(code, 0);
}

// --- a reference on the stack is surplus; in a local it expires

#[test]
fn unused_reference_is_surplus_value_error() {
    let err = check_error("type: V x i64 y i64 ;\n: main ( -- )\n  1 2 V | v |\n  &!v ;\n");
    assert!(
        err.contains("stack effect mismatch") && err.contains("body leaves 1 values"),
        "a leftover reference is an ordinary surplus value: {err}"
    );
    assert!(
        !err.contains("linear"),
        "a reference owns nothing, so this is not a forgotten-disposal error: {err}"
    );
}

#[test]
fn reference_local_expires_without_drop() {
    // `hold`'s `b` is never explicitly dropped, and that is correct: a
    // reference-typed local is never surplus-checked, it just goes out of
    // scope.
    let (stdout, code) = run_src(
        "reference-local-expires",
        "type: V x i64 y i64 ;\n\
         : hold ( &!V -- ) | b | ;\n\
         : main ( -- )\n  1 2 V | v |\n  &!v hold\n  &v &V>x @ . ;\n",
    );
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

// --- exclusivity, in both symmetric directions

#[test]
fn two_live_mutable_borrows_is_error() {
    let err =
        check_error("type: V x i64 y i64 ;\n: main ( -- )\n  1 2 V | v |\n  &!v &!v drop drop ;\n");
    assert!(
        err.contains("`&!v` conflicts with a live borrow of `v`"),
        "expected the exclusivity rejection: {err}"
    );
    assert!(
        err.contains("the mutable borrow taken at line 4, col 3 is still live"),
        "the error should name the borrow it conflicts with: {err}"
    );
    assert!(
        err.contains("line 4, col 7"),
        "the error should locate it: {err}"
    );
    assert!(
        !err.contains("path disjointness"),
        "R7's note belongs only to a conflict with a *projected* borrow: {err}"
    );
}

#[test]
fn shared_borrow_while_mutable_live_is_error() {
    let err =
        check_error("type: V x i64 y i64 ;\n: main ( -- )\n  1 2 V | v |\n  &!v &v drop drop ;\n");
    assert!(
        err.contains("`&v` conflicts with a live borrow of `v`")
            && err.contains("the mutable borrow taken at"),
        "expected the shared-while-mutable rejection: {err}"
    );
}

#[test]
fn mutable_borrow_while_shared_live_is_error() {
    // The direction that is easy to omit: "no `&` while a `&!` is live" has no
    // converse of its own, so this one needs stating outright.
    let err =
        check_error("type: V x i64 y i64 ;\n: main ( -- )\n  1 2 V | v |\n  &v &!v drop drop ;\n");
    assert!(
        err.contains("`&!v` conflicts with a live borrow of `v`")
            && err.contains("the shared borrow taken at"),
        "expected the mutable-while-shared rejection: {err}"
    );
}

#[test]
fn reborrow_while_projected_reference_still_live_is_error() {
    // Both reborrows are individually consumed by their own projection, so a
    // scan for the reborrow's own value would find nothing: the rule is stated
    // over the place, and the derived `&!usize` two steps removed still holds
    // `b` suspended.
    let err = check_error(
        "type: Buf data ^[u8 64] len usize ;\n\
         : two-live ( &!Buf -- )\n  | b |\n  b &!Buf>len\n  b &!Buf>len\n  1 +! 1 +! ;\n",
    );
    assert!(
        err.contains("cannot reborrow `b`") && err.contains("a reference derived from it is live"),
        "expected the suspended-place rejection: {err}"
    );
    assert!(
        err.contains("the derivation taken at line 4"),
        "the error should name the live derivation: {err}"
    );
    assert!(err.contains("line 5"), "the error should locate it: {err}");
}

#[test]
fn two_live_mutable_borrows_to_different_places_is_accepted() {
    // Per place, never a single global counter: `copy-byte` holds a `&!Buf` and
    // a `&Buf` at once, rooted at two different locals.
    let (stdout, code) = run_src(
        "two-places-accepted",
        &format!(
            "{BUFFER_DOGFOOD}\n\
             : main ( -- )\n\
             \x20 new new | a b |\n\
             \x20 &!a 72 >u8 push-byte\n\
             \x20 &!b 90 >u8 push-byte\n\
             \x20 &!a &b 0 copy-byte\n\
             \x20 &a 0 byte-at .\n\
             \x20 &a 1 byte-at .\n\
             \x20 a drop b drop ;\n"
        ),
    );
    assert_eq!(stdout, "72\n90\n");
    assert_eq!(code, 0);
}

#[test]
fn shared_reference_is_copy() {
    // Two live `&V` to one place: shared references carry no exclusivity, so
    // there is nothing for a suspend to protect.
    let (stdout, code) = run_src(
        "shared-reference-is-copy",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  7 8 V | v |\n  &v &v\n  &V>x @ .\n  &V>y @ . ;\n",
    );
    assert_eq!(stdout, "7\n8\n");
    assert_eq!(code, 0);
}

#[test]
fn naming_mutable_reference_local_reborrows() {
    // Naming a `&!` local is a reborrow, not a move: without that a mutable
    // helper would kill its own parameter on first use.
    let (stdout, code) = run_src(
        "reborrow-accepted",
        "type: Counter n i64 ;\n\
         : bump-twice ( &!Counter -- )\n  | c |\n  \
         c &!Counter>n 1 +!\n  c &!Counter>n 1 +! ;\n\
         : main ( -- )\n  0 Counter | k |\n  &!k bump-twice\n  &k &Counter>n @ . ;\n",
    );
    assert_eq!(stdout, "2\n");
    assert_eq!(code, 0);
}

// --- a place stays borrowed until its borrows are consumed

/// A linear place and a word that consumes one: a Copy local is never consumed
/// by being named, so only a linear one can reach the consumption check.
const BOX_PRELUDE: &str = "\
type: Box c ^i64 ;

: sink ( Box -- ) Box> ^> drop ;
";

#[test]
fn move_of_place_borrowed_on_stack_is_error() {
    let err = check_error(&format!(
        "{BOX_PRELUDE}\n: main ( -- )\n  7 ^ Box | b |\n  &b b sink\n  drop ;\n"
    ));
    assert!(
        err.contains("cannot consume the borrowed local `b` of type `Box`"),
        "expected the consume-while-borrowed rejection: {err}"
    );
    assert!(
        err.contains("the shared borrow taken at line 7, col 3 is still live"),
        "the error should name both the place and the borrow: {err}"
    );
}

#[test]
fn move_of_place_borrowed_in_locals_is_error() {
    // The conflicting borrow sits in the locals map rather than on the virtual
    // stack: a reference local is live for the whole block.
    let err = check_error(&format!(
        "{BOX_PRELUDE}\n: main ( -- )\n  7 ^ Box | b |\n  &b | r |\n  b sink ;\n"
    ));
    assert!(
        err.contains("cannot consume the borrowed local `b` of type `Box`")
            && err.contains("still live"),
        "a borrow held in a local must count as live: {err}"
    );
    assert!(err.contains("line 8"), "the error should locate it: {err}");
}

#[test]
fn dispose_of_borrowed_place_is_error() {
    let err = check_error(&format!(
        "{BOX_PRELUDE}\n: main ( -- )\n  7 ^ Box | b |\n  &!b | r |\n  b drop ;\n"
    ));
    assert!(
        err.contains("cannot consume the borrowed local `b` of type `Box`")
            && err.contains("the mutable borrow taken at"),
        "disposing a borrowed place is a consumption like any other: {err}"
    );
}

#[test]
fn move_after_borrow_ends_is_accepted() {
    let (stdout, code) = run_src(
        "move-after-borrow-ends",
        &format!(
            "{BOX_PRELUDE}\n: main ( -- )\n  7 ^ Box | b |\n  \
             &b &Box>c &^ @ .\n  b Box> ^> . ;\n"
        ),
    );
    assert_eq!(stdout, "7\n7\n");
    assert_eq!(code, 0);
}

// --- path disjointness is not modeled

#[test]
fn disjoint_field_borrows_are_conservatively_rejected() {
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  &!v &!V>x\n  &!v &!V>y\n  1 +! 1 +! ;\n",
    );
    assert!(
        err.contains("`&!v` conflicts with a live borrow of `v`"),
        "expected the disjointness rejection: {err}"
    );
    assert!(
        err.contains("path disjointness is not modeled"),
        "the stated limitation should say so outright: {err}"
    );
    assert!(err.contains("line 5"), "the error should locate it: {err}");
}

#[test]
fn sequenced_borrows_of_two_fields_are_accepted() {
    let (stdout, code) = run_src(
        "sequenced-field-borrows",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  &!v &!V>x 10 +!\n  &!v &!V>y 20 +!\n  \
         &v &V>x @ .\n  &v &V>y @ . ;\n",
    );
    assert_eq!(stdout, "11\n22\n");
    assert_eq!(code, 0);
}

// --- two live names for one aggregate place

#[test]
fn mutable_borrow_of_name_aliased_place_is_error() {
    // Naming an aggregate does not copy it: `p` and `q` are two names for one
    // frame slot, so a mutation through `p` would be visible through `q`.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  v v | p q |\n  &!p &!V>x 1 +!\n  q V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `p` mutably") && err.contains("it is aliased by `q`"),
        "expected the aliased-place rejection naming both ends: {err}"
    );
    assert!(
        err.contains("use `dup` for an independent copy"),
        "the error should point at the remedy: {err}"
    );
    assert!(err.contains("line 5"), "the error should locate it: {err}");
}

#[test]
fn mutable_borrow_of_peek_aliased_place_is_error() {
    // The second route: a non-consuming peek pushes the field's interior
    // address, so two peeks of one field alias with no naming involved.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         type: S a V b i64 ;\n\
         : main ( -- )\n  1 2 V 3 S\n  S|>a swap S|>a swap drop\n  | p q |\n  \
         &!p &!V>x 1 +!\n  q V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `p` mutably") && err.contains("it is aliased by `q`"),
        "a peek-aliased place must be rejected too: {err}"
    );
}

#[test]
fn mutable_borrow_of_struct_aliased_by_peeked_field_is_error() {
    // A peeked field is still a name for part of the whole struct, so
    // borrowing the struct while the peek's binding is live must be rejected
    // the same as any other aliasing name — region *overlap*, not equality.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         type: S a V b i64 ;\n\
         : main ( -- )\n  1 2 V 3 S | s |\n  s S|>a | peeked |\n  drop\n  \
         &!s &!S>a &!V>x 40 +!\n  peeked V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `s` mutably") && err.contains("it is aliased by `peeked`"),
        "expected the aliased-place rejection naming both ends: {err}"
    );
}

#[test]
fn mutable_borrow_of_peeked_field_aliased_by_struct_is_error() {
    // The same overlap from the other end: borrowing the field's own name
    // while the struct it was peeked from is still live.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         type: S a V b i64 ;\n\
         : main ( -- )\n  1 2 V 3 S | s |\n  s S|>a | peeked |\n  drop\n  \
         &!peeked &!V>x 40 +!\n  s S>a V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `peeked` mutably") && err.contains("it is aliased by `s`"),
        "expected the aliased-place rejection naming both ends: {err}"
    );
}

#[test]
fn mutable_borrow_aliased_by_if_join_result_is_error() {
    // When both `if` arms leave the *same* place's value (`v` named on
    // both sides, never rebound), the merge must still denote that place's
    // region — collapsing it to `None` regardless of agreement let a name
    // bound to the join alias its source silently.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  1 0 = if v else v end | p |\n  \
         &!v &!V>x 40 +!\n  p V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `v` mutably") && err.contains("it is aliased by `p`"),
        "expected the aliased-place rejection naming both ends: {err}"
    );
}

#[test]
fn mutable_borrow_aliased_by_one_if_arm_only_is_error() {
    // The merge carries the alias forward from the single arm that has one, so
    // the hazard is reported at the borrow even though the other arm leaves an
    // independent value.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  \
         1 0 > if v else v dup swap drop end | p |\n  \
         &!p &!V>x 99 !\n  v V> . .\n  p V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `p` mutably")
            && err.contains("it is aliased by `v`")
            && err.contains("line 5, col 3"),
        "expected the aliased-place rejection naming both ends and locating the borrow: {err}"
    );
}

#[test]
fn mutable_borrow_aliased_by_the_second_if_arm_only_is_error() {
    // The same shape with the arms swapped: arm order must not decide whether
    // the alias survives the merge.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  \
         0 0 > if v dup swap drop else v end | p |\n  \
         &!p &!V>x 99 !\n  v V> . .\n  p V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `p` mutably") && err.contains("it is aliased by `v`"),
        "expected the aliased-place rejection regardless of which arm aliases: {err}"
    );
}

#[test]
fn mutable_borrow_of_a_merge_of_two_aliased_arms_is_error() {
    // The merge keeps both arms' regions, so the borrow is caught even though
    // only one arm's place can be the one the merged value actually denotes.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  3 4 V | w |\n  \
         1 0 > if v else w end | p |\n  \
         &!p &!V>x 99 !\n  v V> . .\n  w V> . .\n  p V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `p` mutably")
            && err.contains("line 6, col 3")
            && err.contains("use `dup`"),
        "expected the borrow-site rejection naming both ends: {err}"
    );
}

#[test]
fn mutable_borrow_of_a_place_a_merge_may_denote_is_error() {
    // The symmetric direction: borrowing the place rather than the merge. The
    // merge is only aliased with `w` on the path that was not taken, which is
    // exactly why the region cannot be dropped at the join.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  3 4 V | w |\n  \
         0 0 > if v else w end | p |\n  \
         &!w &!V>x 99 !\n  p V> drop .\n  w V> drop . ;\n",
    );
    assert!(
        err.contains("cannot borrow `w` mutably") && err.contains("it is aliased by `p`"),
        "a merge keeps every region either arm could have left: {err}"
    );
}

#[test]
fn if_join_of_two_named_aggregates_without_a_borrow_is_accepted() {
    // Selecting one of two owned records takes no borrow, so the aliasing rule
    // has nothing to say about it. Rejecting this at the join was tried and is
    // too blunt: it forces a copy on a program that never mutates.
    let (stdout, code) = run_src(
        "join-no-borrow",
        "type: V x i64 y i64 ;\n\
         : bigger ( V V -- V ) | a b |\n  \
         a V> drop b V> drop > if a else b end ;\n\
         : main ( -- ) 1 2 V 5 6 V bigger V> . . ;\n",
    );
    assert_eq!(stdout, "6\n5\n", "the larger record's fields, unchanged");
    assert_eq!(code, 0);
}

#[test]
fn mutable_borrow_of_a_place_over_duplicated_is_error() {
    // `over` reuses its operand rather than deep-copying it, so both slots
    // denote one address even when neither has been named yet.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V 7 over | a b c |\n  \
         &!a &!V>x 99 !\n  a V> . .\n  c V> . .\n  b . ;\n",
    );
    assert!(
        err.contains("cannot borrow `a` mutably") && err.contains("it is aliased by `c`"),
        "an `over` of an anonymous aggregate leaves two names for one address: {err}"
    );
}

#[test]
fn mutable_borrow_of_an_array_over_duplicated_is_error() {
    // Same hole reached through an array rather than a struct.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V 4 fill 7 over | arr n arr2 |\n  \
         &!arr 0 &!> &!V>x 99 !\n  &arr 0 &> @ V> drop .\n  \
         &arr2 0 &> @ V> drop .\n  n . ;\n",
    );
    assert!(
        err.contains("cannot borrow `arr` mutably") && err.contains("it is aliased by `arr2`"),
        "an `over` of an anonymous array leaves two names for one address: {err}"
    );
}

#[test]
fn over_of_an_aggregate_without_a_mutable_borrow_is_accepted() {
    // The rule still fires at the borrow, not at the duplication: two names for
    // a value nothing mutates read identically.
    let (stdout, code) = run_src(
        "over-no-borrow",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V 7 over | a b c |\n  \
         a V> . .\n  c V> . .\n  b . ;\n",
    );
    assert_eq!(stdout, "2\n1\n2\n1\n7\n");
    assert_eq!(code, 0);
}

#[test]
fn mutable_borrow_of_a_place_a_merged_peek_may_denote_is_error() {
    // A projection out of a merged value must project the field out of every
    // region the merge could denote. Dropping the merged parent leaves only the
    // peeked field, so nothing but that projection can catch the borrow.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         type: S a V b i64 ;\n\
         : main ( -- )\n  1 2 V 7 S | s |\n  3 4 V 8 S | t |\n  \
         0 0 > if s else t end S|>a swap drop | inner |\n  \
         &!t &!S>a &!V>x 99 !\n  inner V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `t` mutably") && err.contains("it is aliased by `inner`"),
        "a peek out of a merge must keep every arm's field region: {err}"
    );
}

#[test]
fn dup_makes_aliased_names_independent() {
    // `dup` is the whole remedy, and not a new concept: it is the language's
    // existing explicit copy, applied to a case that currently slips past.
    let (stdout, code) = run_src(
        "dup-makes-independent",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V dup | p q |\n  &!p &!V>x 40 +!\n  p V> . .\n  q V> . . ;\n",
    );
    assert_eq!(
        stdout, "2\n41\n2\n1\n",
        "the duped copy must be independent of the mutated original"
    );
    assert_eq!(code, 0);
}

#[test]
fn repeated_naming_without_mutable_borrow_is_accepted() {
    // The rule fires at the borrow, not at the naming: two names for a value
    // nothing mutates read identically, which is why `examples/vm.sth` (which
    // names `vm` 38 times and never takes a `&!`) is untouched by this slice.
    let (stdout, code) = run_src(
        "repeated-naming-accepted",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  v v | p q |\n  p V> . .\n  q V> . . ;\n",
    );
    assert_eq!(stdout, "2\n1\n2\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn mutable_borrow_of_place_aliased_on_the_stack_is_error() {
    // The alias need not be *bound*: a concatenative body leaves aggregates on
    // the virtual stack constantly, so an unnamed naming still sitting there is
    // the common shape of this hazard. It has no name to cite, so the error
    // locates it by the site that pushed it.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  v\n  &!v &!V>x 40 +!\n  V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `v` mutably") && err.contains("a value on the stack"),
        "a stack-resident alias must be rejected too: {err}"
    );
    assert!(
        err.contains("pushed at line 4"),
        "the error should locate the alias it cannot name: {err}"
    );
}

#[test]
fn mutable_borrow_of_struct_aliased_by_peek_on_the_stack_is_error() {
    // The peek route with neither end bound. The parent copy `s` leaves behind
    // is dropped, so the only thing left overlapping is the peeked interior
    // itself and the diagnostic has to locate the peek rather than the naming.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         type: S a V b i64 ;\n\
         : main ( -- )\n  1 2 V 3 S | s |\n  s S|>a swap drop\n  \
         &!s &!S>a &!V>x 40 +!\n  V> . . ;\n",
    );
    assert!(
        err.contains("cannot borrow `s` mutably")
            && err.contains("a value on the stack")
            && err.contains("col 5"),
        "a peeked interior left on the stack still aliases its parent: {err}"
    );
}

#[test]
fn naming_a_place_while_mutably_borrowed_is_error() {
    // The symmetric direction, the one an exclusivity rule makes easy to omit: checking
    // only at the borrow catches `v ... &!v` and misses `&!v ... v`, which is
    // the same hazard with the two terms swapped.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  &!v\n  v | p |\n  \
         &!V>x 40 +!\n  p V> . . ;\n",
    );
    assert!(
        err.contains("cannot name `v`") && err.contains("a mutable borrow of it is still live"),
        "expected the naming-side rejection naming both ends: {err}"
    );
    assert!(
        err.contains("line 5") && err.contains("line 4"),
        "the error should locate both the naming and the borrow: {err}"
    );
}

#[test]
fn naming_a_place_whose_mutable_borrow_is_bound_is_error() {
    // The naming side reads bindings as well as the stack: a `&!` bound into a
    // local is live for the whole body, so no naming after it is safe.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  &!v | r |\n  v | p |\n  \
         r &!V>x 40 +!\n  p V> . . ;\n",
    );
    assert!(
        err.contains("cannot name `v`") && err.contains("a mutable borrow of it is still live"),
        "a borrow bound into a local must block naming its place: {err}"
    );
}

#[test]
fn naming_a_place_after_its_borrow_ends_is_accepted() {
    // The guard against an over-broad naming-side rule: the borrow is live
    // until the term that consumes its slot, and `+!` is that term, so
    // naming `v` afterwards is ordinary reuse.
    let (stdout, code) = run_src(
        "naming-after-borrow-ends",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  &!v &!V>x 40 +!\n  v V> . . ;\n",
    );
    assert_eq!(stdout, "2\n41\n");
    assert_eq!(code, 0);
}

#[test]
fn dup_makes_a_stack_alias_independent() {
    // `dup` is the remedy for the stack route too: the copy denotes a region of
    // its own, so keeping it and dropping the original naming leaves the
    // snapshot readable across the mutation.
    let (stdout, code) = run_src(
        "dup-makes-stack-alias-independent",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  v dup swap drop\n  \
         &!v &!V>x 40 +!\n  V> . .\n  v V> . . ;\n",
    );
    assert_eq!(
        stdout, "2\n1\n2\n41\n",
        "the duped snapshot must not see the mutation"
    );
    assert_eq!(code, 0);
}

// --- loop back-edge rules, both sides

const BIG_LIST: &str = "\
type: List | Nil | Cons v i64 next ^List ;
\
: push-front ( List i64 -- List )\n  | rest v |\n  v rest ^ Cons ;
\
: build ( i64 List -- List )\n  | n acc |\n  n 0 = if\n    acc\n  else\n    \
n 1 - acc n push-front build\n  end ;
\
: walk ( &!List -- )\n  | Nil\n  | Cons | v next |\n      v 1 +!\n      next &!^ walk\n  ;
";

#[test]
fn reference_parameter_crosses_back_edge_in_constant_stack() {
    // The accept-case: `walk`'s own reference parameter, reborrowed from a
    // `Cons` payload projection each iteration, crosses the self-tail-call
    // back-edge a million times in constant stack (no growth, no overflow),
    // mutating every node in place; the front node's value, read back after
    // the call returns, proves the mutation actually landed rather than the
    // loop silently no-op-ing.
    let src = format!(
        "{BIG_LIST}\
         type: Popped rest List val i64 ;\n\
         : pop ( List -- Popped )\n  | Nil   Nil 0 Popped\n  | Cons  | v next | next ^> v Popped\n  ;\n\
         : main ( -- )\n  1000000 Nil build\n  | l |\n  &!l walk\n  l pop Popped>\n  . drop ;\n",
    );
    let (stdout, code) = run_src("ref-param-crosses-back-edge", &src);
    assert_eq!(
        stdout, "2\n",
        "a million-node walk must increment the front value exactly once, in constant stack"
    );
    assert_eq!(code, 0);
}

#[test]
fn reference_to_local_across_back_edge_is_error() {
    // The rejection: `x` is a local *created this iteration*, not the
    // parameter `r` or anything projected from it, so a reference to it
    // cannot legally cross the back-edge — its storage does not survive to
    // the next iteration (locals rebind at the loop header).
    let err = check_error(
        "type: V x i64 ;\n\
         : spin ( &!V i64 -- )\n  | r n |\n  n 0 = if\n  else\n    \
         0 V | x |\n    &!x n 1 - spin\n  end ;\n\
         : main ( -- )\n  0 V | v |\n  &!v 3 spin\n  v drop ;\n",
    );
    assert!(
        err.contains("a reference to a local cannot cross a loop"),
        "unexpected message: {err}"
    );
    assert!(err.contains('x'), "the error should name the local: {err}");
}

#[test]
fn borrowed_local_carried_across_back_edge_is_error() {
    // The other half of the loop justification: an owned local that is still
    // borrowed cannot be loop-carried either. This is the existing
    // naming-side rule (`naming_a_place_while_mutably_borrowed_is_error`),
    // which fires here just as it would anywhere else — the hazard a
    // self-tail-recursive loop would otherwise let through is exactly the
    // one that rule already closes.
    // The borrow is bound so both arms leave the same depth: the program is
    // well-formed apart from the borrow, so the rejection cannot be coming
    // from a stack-effect mismatch instead.
    let err = check_error(
        "type: V x i64 ;\n\
         : spin ( V i64 -- V )\n  | acc n |\n  n 0 = if\n    acc\n  else\n    \
         &!acc | r |\n    acc n 1 - spin\n  end ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("cannot name `acc`")
            && err.contains("a mutable borrow of it is still live (line 7, col 5)"),
        "unexpected message: {err}"
    );
}

// --- branch-join borrow-state agreement

#[test]
fn borrow_on_one_arm_only_is_error() {
    // Both arms leave a stack of identical shape (a `&!i64`), but each
    // suspends a *different* place: type unification alone has nothing to say
    // about that, so it must be rejected as a disagreement at the join.
    let err = check_error(
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  1 3 V | w |\n  true if\n    &!v\n  else\n    &!w\n  end\n  \
         &!V>x 1 +!\n  v drop\n  w drop ;\n",
    );
    assert!(
        err.contains("borrow state disagrees"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains('v') && err.contains('w'),
        "the error should name both arms' places: {err}"
    );
}

#[test]
fn borrow_live_on_both_arms_is_accepted() {
    // Both arms suspend the *same* place: there is nothing to reject, and the
    // merged reference stays usable past the join.
    let (stdout, code) = run_src(
        "borrow-live-on-both-arms",
        "type: V x i64 y i64 ;\n\
         : main ( -- )\n  1 2 V | v |\n  true if\n    &!v\n  else\n    &!v\n  end\n  \
         &!V>x 1 +!\n  v V> . . ;\n",
    );
    assert_eq!(stdout, "2\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn borrow_join_disagreeing_on_reborrowed_parameter_is_error() {
    // The two arms reborrow *different* reference parameters, so neither
    // derivation has an owned root in this frame — yet each suspends its own
    // reference local, and the merge keeps only one. Without rejecting it,
    // the `else` path reaches `q &!Buf>len` with a `&!usize` derived from `q`
    // still live: the two-live-mutable-references hazard the suspend rule
    // exists to stop.
    let err = check_error(
        "type: Buf  data ^[u8 64]  len usize ;\n\
         : two-parents ( &!Buf &!Buf -- )\n  | p q |\n  \
         true if p else q end\n  &!Buf>len\n  q &!Buf>len 1 +!\n  1 +! ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("borrow state disagrees"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("reborrow of `p`") && err.contains("reborrow of `q`"),
        "the error should name both arms' reborrowed places: {err}"
    );
}

// --- reference-mode enum elimination

#[test]
fn reference_mode_clause_binds_payload_as_reference() {
    // A word whose declared top input is `&Enum` dispatches clause-style in
    // reference mode: `v`'s payload binding is a `&i64`, fetched with `@`
    // rather than moved, and the recursion projects a fresh `&List` through
    // the cell each iteration — the same shape as `walk`, shared instead of
    // mutable.
    let (stdout, code) = run_src(
        "reference-mode-clause-binds-payload",
        "type: List | Nil | Cons v i64 next ^List ;\n\
         : sum ( i64 &List -- i64 )\n  | Nil | acc | acc\n  | Cons | acc v next |\n      \
         acc v @ +\n      next &^ sum\n  ;\n\
         : push-front ( List i64 -- List )\n  | rest v |\n  v rest ^ Cons ;\n\
         : build ( i64 List -- List )\n  | n acc |\n  n 0 = if\n    acc\n  else\n    \
         n 1 - acc n push-front build\n  end ;\n\
         : main ( -- )\n  5 Nil build\n  | l |\n  0 &l sum .\n  l drop ;\n",
    );
    assert_eq!(stdout, "15\n");
    assert_eq!(code, 0);
}

#[test]
fn reference_mode_clause_payload_bindings_are_simultaneously_live() {
    // The clause form's named exemption: a clause binds every field of one variant
    // at once, with no root local to reborrow from, and the fields are
    // statically disjoint — so both payload references may be live together,
    // which the general disjointness rule would reject for two projections of one place.
    let (stdout, code) = run_src(
        "reference-mode-clause-disjoint-payloads",
        "type: P | Zero | Both a i64 b i64 ;\n\
         : bump ( &!P -- )\n  | Zero\n  | Both | a b |\n      a b\n      2 +!\n      1 +! ;\n\
         : show ( P -- )\n  | Zero\n  | Both | a b | a . b .\n  ;\n\
         : main ( -- )\n  1 2 Both | p |\n  &!p bump\n  p show ;\n",
    );
    assert_eq!(
        stdout, "2\n4\n",
        "each field is incremented through its own reference"
    );
    assert_eq!(code, 0);
}

#[test]
fn reference_mode_clause_fetching_linear_payload_is_error() {
    // No clause may consume a payload binding: fetching `next`'s referent
    // (`^List`, always linear) is the same rejection a fetched/stored
    // linear `T` gets anywhere else, not a special reference-mode rule.
    let err = check_error(
        "type: List | Nil | Cons v i64 next ^List ;\n\
         : bad ( &!List -- )\n  | Nil\n  | Cons | v next |\n      next @\n      v drop\n  ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("cannot access the linear referent"),
        "unexpected message: {err}"
    );
}

// --- the full dogfood

#[test]
fn reference_dogfood_prints_expected_bytes() {
    let src = std::fs::read_to_string("examples/refs.sth").expect("the dogfood file should exist");
    let (stdout, code) = run_src("reference-dogfood", &src);
    assert_eq!(
        stdout, "72\n90\n2\n2\n",
        "push-byte's write, copy-byte's copy, the buffer's length, and walk's incremented head"
    );
    assert_eq!(code, 0);
}
