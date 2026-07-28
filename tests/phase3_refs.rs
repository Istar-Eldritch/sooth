//! Phase 3 Slice 6 goldens: reference types, places, projection and access.
//!
//! Kept out of `tests/phase0.rs` deliberately: criterion 16 asserts that file
//! is never modified from the slice's base commit, so a new golden belongs
//! somewhere the addition-only check has nothing to reason about.

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

// --- criterion 1: `&`/`&!` are prefix borrows of a place, and only of a place

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

// --- criterion 2: only an aggregate local is a borrow root; `&T` is Copy

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
fn projection_to_scalar_field_is_accepted() {
    // R11 rejects a scalar *root*, not a scalar *result*: the referent here is
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
    // Two live `&!` to one place, rejected by `dup`'s existing Copy gate:
    // `&!T` is not `Copy`.
    let err = check_error("type: V x i64 y i64 ;\n: main ( -- )\n  1 2 V | v |\n  &!v dup ;\n");
    assert!(
        err.contains("`dup`") && err.contains("&!V"),
        "expected `dup` to reject a mutable reference: {err}"
    );
}

// --- criterion 3: projection through all three shapes, one spelling per
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

// --- criterion 4: `@`, `!`, `+!`

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
    // R4's Copy restriction is Copy-vs-linear, never scalar-vs-aggregate: a
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

// --- criterion 5: escape is prevented structurally

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

// --- criterion 13: `&!T` lowers to an opaque pointer, not a by-value aggregate

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

// --- criterion 15: a reference on the stack is surplus; in a local it expires

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
