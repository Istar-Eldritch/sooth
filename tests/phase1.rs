//! Phase 1 golden session tests: spawn the `repl` binary, pipe a scripted
//! stdin session, and assert on stdout. Each test is one exit criterion.

use std::io::Write;
use std::process::{Command, Stdio};

mod common;

/// Run a scripted REPL session (one input line per element of `lines`) and
/// return the whole captured stdout.
fn run_session(lines: &[&str]) -> String {
    run_session_traced(lines, false)
}

/// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
/// primitive in Slice 8c: an ordinary one-field struct with a `drop`
/// overload, so it is linear for the same reason any resource is, not by
/// any compiler-known bit. Defining it as two REPL lines emits "defined type
/// Spy" and "defined drop for Spy", two lines every golden below that uses
/// it must account for; unlike the retired scalar primitive, a bare `Spy`
/// residual now shows on the REPL stack as the aggregate placeholder
/// `<Spy>`, not its tag value.
const SPY_TYPE_LINE: &str = "type: Spy tag i64 ;";
const SPY_DROP_LINE: &str = ": drop ( Spy -- )  | s | \"drop \" . s Spy> . ;";

/// Run a scripted session with the allocation trace enabled or disabled (R10).
/// The trace shares the session's stdout, so an allocation-observing session
/// reads as one transcript: `alloc <size>`/`free <size>` lines interleaved with
/// the REPL's own `defined`/`stack:` output.
fn run_session_traced(lines: &[&str], trace: bool) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sooth"));
    // The gate is set or cleared explicitly, so an ambient value in the caller's
    // environment can neither hide a trace nor add one.
    match trace {
        true => cmd.env(sooth::ir::TRACE_ALLOC_ENV, "1"),
        false => cmd.env_remove(sooth::ir::TRACE_ALLOC_ENV),
    };
    let mut child = cmd
        .arg("repl")
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
    drop(stdin); // close stdin so the REPL sees EOF and exits

    let output = child.wait_with_output().expect("repl should exit cleanly");
    assert!(
        output.status.success(),
        "repl exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

#[test]
fn alloc_trace_stays_empty_in_a_session_that_never_allocates() {
    // Each REPL `.so` carries its own copy of the allocator shim and trace, which
    // is benign for the same reason the spy's copies are: they wrap libc and hold
    // no state, and the trace's state lives in stdout, not in the module. So a
    // session that constructs no cell prints no trace even with the gate on.
    let out = run_session_traced(&[": sq ( i64 -- i64 ) | n | n n mul ;", "5 sq"], true);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["defined sq", "stack: 25"]);
}

#[test]
fn define_then_call_across_lines() {
    let out = run_session(&[": sq ( i64 -- i64 ) | n | n n mul ;", "5 sq"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["defined sq", "stack: 25"]);
}

#[test]
fn stack_persists_across_lines() {
    let out = run_session(&[": sq ( i64 -- i64 ) | n | n n mul ;", "5", "sq", "1 add"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["defined sq", "stack: 5", "stack: 25", "stack: 26"]
    );
}

#[test]
fn redefinition_takes_effect_for_later_lines() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) | n | n n mul ;",
        "3 sq",
        ": sq ( i64 -- i64 ) | n | n n n mul mul ;",
        "3 sq",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["defined sq", "stack: 9", "defined sq", "stack: 9 27"]
    );
}

#[test]
fn bad_line_reports_and_session_survives() {
    let out = run_session(&["5", "unknown-word", "1 add"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "stack: 5");
    assert!(
        lines[1].contains("unknown word") && lines[1].contains("unknown-word"),
        "expected an unknown-word diagnostic naming `unknown-word`: {}",
        lines[1]
    );
    assert_eq!(lines[2], "stack: 6");
}

#[test]
fn type_error_line_reports_and_session_survives() {
    let out = run_session(&["5", "True 1 add", "1 add"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "stack: 5");
    assert!(
        lines[1].contains("`i64`") && lines[1].contains("`Bool`"),
        "expected a type-mismatch diagnostic: {}",
        lines[1]
    );
    assert_eq!(lines[2], "stack: 6");
}

#[test]
fn failed_redefinition_keeps_old_generation_resident() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) | n | n n mul ;",
        "3 sq",
        ": sq ( i64 -- i64 ) dup dup mul ;",
        "3 sq",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 6, "unexpected output:\n{out}");
    assert_eq!(lines[0], "defined sq");
    assert_eq!(lines[1], "stack: 9");
    assert!(
        lines[2].contains("stack effect mismatch in `sq`"),
        "expected a check error for the bad redefinition: {}",
        lines[2]
    );
    assert!(lines[3].contains("body leaves 2 values"));
    assert!(lines[3].contains("declares 1 outputs"));
    // The failed redefinition never committed: `sq` still resolves to the
    // original generation (`n n mul`), and the stack from the first `3 sq` is
    // untouched, so the second `3 sq` appends its own 9.
    assert_eq!(lines[5], "stack: 9 9");
}

#[ignore = "REPL trait/impl checking is unimplemented: `check.rs`'s two \
    REPL check sites (`:1293`/`:1387`) hardcode `TraitResolveCtx::scratch()`, \
    whose premise (a session declares no `trait:`, so no `Bound::User` \
    reaches a REPL body) is false once a session imports `core::cmp`. A \
    comparison call then indexes past the scratch trait table and ICEs at \
    `check/poly.rs:976`. Fixing it needs a `Session`-level traits/impls \
    accumulation table (Session has none, unlike its `structs`/`enums`) \
    threaded through both sites: tracked as the REPL trait/impl slice."]
#[test]
fn sign_definable_and_callable_in_repl() {
    // P8.S2 (R3): the typed core is imported, not seeded -- a session names
    // it exactly as a file does.
    let cmp = common::repl_core_import("cmp", "gt");
    let boolean = common::repl_core_import("bool", "if");
    let out = run_session(&[
        &cmp,
        &boolean,
        ": sign ( i64 -- i64 ) 0 gt ~[ 1 ] ~[ 0 ] if ;",
        "-7 sign",
        "7 sign",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "imported cmp",
            "imported bool",
            "defined sign",
            "stack: 0",
            "stack: 0 1"
        ]
    );
}

#[test]
fn bool_residual_displays_as_true_or_false() {
    // Matches `.`'s print semantics: `true`/`false`, not the raw 0/1.
    let out = run_session(&["True", "False"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["stack: True", "stack: True False"]);
}

#[test]
fn calculator_session_dogfood() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) | n | n n mul ;",
        ": neg ( i64 -- i64 ) 0 swap sub ;",
        "3 sq",
        "neg",
        "10 add",
        "2 mul",
        ".",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined sq",
            "defined neg",
            "stack: 9",
            "stack: -9",
            "stack: 1",
            "stack: 2",
            "2",
            "stack: (empty)",
        ]
    );
}

/// S5: a sub-word (`u8`) value survives a line boundary on the carried stack
/// and is used correctly on the next line, proving Q2 (the carried buffer
/// slot stays 8 bytes wide and is canonicalized/relabeled on use).
#[test]
fn subword_carried_value_survives_line_boundary() {
    // Wraps (200 + 100 = 300, mod 256 = 44): an in-range case would pass even
    // if the carried slot were never canonicalized on reload, so this pins the
    // actual point of R16/Q2, that the carried `u8` is relabeled/canonicalized
    // across the line boundary, not just carried as an opaque 8-byte value.
    let out = run_session(&["200 >u8", "100 >u8 add", ">i64 ."]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["stack: 200", "stack: 44", "44", "stack: (empty)"]
    );
}

/// S6: a carried `f64` survives a line boundary and is used correctly on the
/// next line (R20 marshalling), and displays as its float value rather than
/// its `i64` bit pattern (R21). The mid-session `stack: 3.5` / `stack: 4.5`
/// lines prove the display path; the second line consuming the carried 3.5
/// proves the float re-enters as a true float, not a stale integer.
#[test]
fn carried_float_survives_line_boundary_and_displays_as_float() {
    let out = run_session(&["1.5 2.0 add", "1.0 add", "."]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["stack: 3.5", "stack: 4.5", "4.5", "stack: (empty)"]
    );
}

/// S5/R16-R18: a struct value survives a REPL line boundary on the size-aware
/// carried stack, is used (a field read) on the next line, and displays as its
/// `<TypeName>` placeholder (M4) rather than field bytes.
#[test]
fn carried_struct_survives_line_boundary() {
    let out = run_session(&["type: Vec2 x i64 y i64 ;", "5 6 Vec2", "&x @ . drop"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["defined type Vec2", "stack: <Vec2>", "5", "stack: (empty)"]
    );
}

/// R18: a scalar slot sitting past a struct slot on the carried stack keeps a
/// correct byte offset (the struct spans two 8-byte cells, so the `99` is read
/// from the cell after it, not `index * 8`). The struct's field is still
/// readable on the following line after the scalar is dropped.
#[test]
fn carried_struct_and_scalar_offsets_stay_correct() {
    let out = run_session(&[
        "type: Vec2 x i64 y i64 ;",
        "5 6 Vec2 99",
        "drop &y @ . drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Vec2",
            "stack: <Vec2> 99",
            "6",
            "stack: (empty)",
        ]
    );
}

/// A struct whose aggregate size is not a multiple of 8 (`Pair`, two `i8`
/// fields, 2 bytes) still round-trips across a REPL line boundary: the
/// carried cell count rounds up to one 8-byte cell, exercised here at
/// runtime rather than only by the `carried_slot_bytes` unit test.
#[test]
fn carried_struct_with_non_eight_multiple_size_survives_line_boundary() {
    let out = run_session(&[
        "type: Pair a i8 b i8 ;",
        "1 >i8 2 >i8 Pair",
        "&a @ . &b @ . drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Pair",
            "stack: <Pair>",
            "1",
            "2",
            "stack: (empty)"
        ]
    );
}

/// A duplicate `type:` in the REPL is a located error (X2) and rolls back:
/// the original struct stays usable, and a recursive `type:` is reported
/// rather than hanging (X3, M5). Both leave the session intact.
#[test]
fn struct_declaration_errors_report_and_session_survives() {
    let out = run_session(&[
        "type: Vec2 x i64 y i64 ;",
        "type: Vec2 a i64 ;",
        "type: Loop next Loop ;",
        "5 6 Vec2 &y @ . drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "defined type Vec2");
    assert!(
        lines[1].contains("duplicate type `Vec2`"),
        "expected a duplicate-type diagnostic naming `Vec2`: {}",
        lines[1]
    );
    assert!(
        lines[2].contains("recursive struct definition") && lines[2].contains("Loop"),
        "expected a recursive-struct diagnostic naming the cycle: {}",
        lines[2]
    );
    // The rolled-back duplicate never shadowed the original: `Vec2>y` of
    // `(5, 6)` still yields 6.
    assert_eq!(lines[3], "6");
    assert_eq!(lines[4], "stack: (empty)");
}

/// S8: the `examples/vectors.sth` dogfood, defined and run across REPL lines
/// (word definitions carrying struct types across the line boundary, then
/// two calls exercising the nested `Segment>`/`vec2-sub`/`len2` span and the
/// `shift-x` functional setter).
#[test]
fn vectors_dogfood_runs_in_repl() {
    let out = run_session(&[
        "type: Vec2 x i64 y i64 ;",
        "type: Segment from Vec2 to Vec2 ;",
        ": vec2-sub ( Vec2 Vec2 -- Vec2 ) | a b | a &x @ swap drop b &x @ swap drop sub a &y @ swap drop b &y @ swap drop sub Vec2 ;",
        ": len2 ( Vec2 -- i64 ) | v | v &x @ swap drop v &x @ swap drop mul v &y @ swap drop v &y @ swap drop mul add ;",
        ": span ( Segment -- Vec2 ) Segment> swap vec2-sub ;",
        ": shift-x ( Vec2 i64 -- Vec2 ) | v dx | v &x @ swap drop dx add | newx | v &!x newx ! ;",
        "0 0 Vec2 3 4 Vec2 Segment span len2 .",
        "5 6 Vec2 1 shift-x &x @ . drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Vec2",
            "defined type Segment",
            "defined vec2-sub",
            "defined len2",
            "defined span",
            "defined shift-x",
            "25",
            "stack: (empty)",
            "6",
            "stack: (empty)",
        ]
    );
}

/// Guards the flush-before-call discipline the spec flags as a determinism
/// risk: the host's stdout buffer and the loaded code's C stdio buffer must
/// both be flushed so `.` output lands before the next `stack:` line, in
/// program order, on every run.
#[test]
fn dot_output_interleaves_before_stack() {
    let out = run_session(&["5 .", "2 3 add ."]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["5", "stack: (empty)", "5", "stack: (empty)"]);
}

/// Slice 4 (criterion 7, Phase 3 slice): an enum value is constructed at REPL
/// scope, marshalled into the carried buffer, and displayed as its
/// `<TypeName>` placeholder (M4). A multi-field float variant, a zero-field
/// variant, and a one-field variant all construct; the `<Shape>` slot then
/// survives a later line's boundary (a scalar pushed on top reads the cell
/// *past* the enum's multi-cell slot, so a mis-sized marshalling would
/// corrupt it).
#[test]
fn enum_constructs_and_displays_placeholder_across_lines() {
    let out = run_session(&[
        "type: Shape | Circle r f64 | Rect w f64 h f64 ;",
        "type: MaybeInt | None | Some v i64 ;",
        "2.0 Circle",
        "3.0 4.0 Rect",
        "None",
        "7 Some",
        "99",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Shape",
            "defined type MaybeInt",
            "stack: <Shape>",
            "stack: <Shape> <Shape>",
            "stack: <Shape> <Shape> <MaybeInt>",
            "stack: <Shape> <Shape> <MaybeInt> <MaybeInt>",
            "stack: <Shape> <Shape> <MaybeInt> <MaybeInt> 99",
        ]
    );
}

/// Criterion 7: an enum is declared on one line, then a word eliminating it is
/// defined on a *later* line (R18's variant-set seeding from `Session.enums`,
/// since the parser pre-pass alone only scans the current unit). A value
/// constructs and displays `<Shape>`, then a further line eliminates it
/// through that word.
#[test]
fn enum_declared_then_eliminating_word_defined_on_later_lines() {
    let out = run_session(&[
        "type: Shape | Circle r f64 | Rect w f64 h f64 ;",
        ": area ( Shape -- f64 ) ~[ ( Circle ) Circle> dup mul 3.14159 mul ] ~[ ( Rect ) Rect> | w h | w h mul ] Shape? ;",
        "2.0 Circle",
        "area .",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Shape",
            "defined area",
            "stack: <Shape>",
            "12.5664",
            "stack: (empty)",
        ]
    );
}

/// Criterion 8: `examples/shapes.sth` runs correctly end-to-end *in the REPL*,
/// not just natively. Each definition collapses to one REPL line (multi-line
/// input isn't the point of this golden; exercising every eliminator arm from
/// the dogfood file is), and all four `main` operations run, hitting the exact
/// native golden's output (`12.5664 / 12 / 5 / 7`) through the REPL path.
#[test]
fn shapes_dogfood_runs_full_program_in_repl() {
    let out = run_session(&[
        "type: Shape | Circle r f64 | Rect w f64 h f64 ;",
        "type: MaybeInt | None | Some v i64 ;",
        ": area ( Shape -- f64 ) ~[ ( Circle ) Circle> dup mul 3.14159 mul ] ~[ ( Rect ) Rect> | w h | w h mul ] Shape? ;",
        ": unwrap-or ( i64 MaybeInt -- i64 ) ~[ ( None ) drop ] ~[ ( Some ) Some> swap drop ] MaybeInt? ;",
        "2.0 Circle area .",
        "3.0 4.0 Rect area .",
        "5 None unwrap-or .",
        "5 7 Some unwrap-or .",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Shape",
            "defined type MaybeInt",
            "defined area",
            "defined unwrap-or",
            "12.5664",
            "stack: (empty)",
            "12",
            "stack: (empty)",
            "5",
            "stack: (empty)",
            "7",
            "stack: (empty)",
        ]
    );
}

// Slice 5 (fixed-size arrays + `usize`): criterion 8, REPL parity (R22/R23).

/// An array constructs at REPL scope, marshals into the carried buffer
/// across the line boundary, and a residual array slot renders `<[T N]>`
/// (D10); a `usize` slot prints via the type-directed `.`, same as any other
/// carried scalar.
#[test]
fn array_and_usize_cross_repl_line_boundary_and_render() {
    let out = run_session(&["0 4 fill", "5 >usize"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["stack: <[i64 4]>", "stack: <[i64 4]> 5"]);
}

/// Criterion 8: a REPL-scope stack exercise — array-as-struct-field, a
/// runtime `usize` cursor (the trap path), in-place mutation through `&!>`,
/// a non-consuming read through `&>`/`@`, and `len` — with its words declared
/// and run entirely across REPL lines (`type:` with an array field, then
/// `fill`/`&!>`/`&>`/`len` at REPL scope). It stresses the same features as
/// `examples/stack.sth` but defines its own words inline rather than loading
/// that file.
#[test]
fn stack_dogfood_runs_in_repl() {
    let out = run_session(&[
        "type: Stack items [i64 16] top usize ;",
        "type: Popped rest Stack item i64 ;",
        ": empty ( -- Stack ) 0 16 fill 0 >usize Stack ;",
        ": push ( Stack i64 -- Stack ) | s x | s &top @ swap drop | i | &!s &!items i &!> x ! s &top @ swap drop 1 add | newtop | s &!top newtop ! ;",
        ": pop ( Stack -- Popped ) | s | s &top @ swap drop 1 sub | i | &s &items i &> @ | v | s &!top i ! v Popped ;",
        ": peek ( Stack -- Popped ) | s | s &top @ swap drop 1 sub | i | &s &items i &> @ | v | s v Popped ;",
        "empty 1 push 2 push 3 push",
        "peek Popped> .",
        "pop Popped> .",
        "pop Popped> .",
        "peek Popped> .",
        "&items @ swap drop len .",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Stack",
            "defined type Popped",
            "defined empty",
            "defined push",
            "defined pop",
            "defined peek",
            "stack: <Stack>",
            "3",
            "stack: <Stack>",
            "3",
            "stack: <Stack>",
            "2",
            "stack: <Stack>",
            "1",
            "stack: <Stack>",
            "16",
            "stack: <[i64 16]>",
            "stack: (empty)",
        ]
    );
}

/// A large-payload variant (three `i64` fields, exceeding one 8-byte carried
/// cell) survives a REPL line boundary intact: its multi-cell slot is blitted
/// out of and back into the buffer, and a scalar pushed afterward still reads
/// its own cell rather than the enum's payload.
#[test]
fn enum_large_payload_survives_line_boundary() {
    let out = run_session(&["type: Big | B a i64 b i64 c i64 ;", "1 2 3 B", "42"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["defined type Big", "stack: <Big>", "stack: <Big> 42"]
    );
}

/// A duplicate enum name (X2) and a recursive enum (X3, M5) are located
/// errors that roll back, leaving the session usable; the recursive case
/// reports rather than hanging.
#[test]
fn enum_declaration_errors_report_and_session_survives() {
    let out = run_session(&[
        "type: Shape | Circle r f64 | Rect w f64 h f64 ;",
        "type: Shape | A ;",
        "type: Loop | Wrap next Loop ;",
        "2.0 Circle",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "defined type Shape");
    assert!(
        lines[1].contains("duplicate type `Shape`"),
        "expected a duplicate-type diagnostic naming `Shape`: {}",
        lines[1]
    );
    assert!(
        lines[2].contains("recursive enum definition") && lines[2].contains("Loop"),
        "expected a recursive-enum diagnostic naming the cycle: {}",
        lines[2]
    );
    // The rolled-back duplicate never shadowed the original Shape.
    assert_eq!(lines[3], "stack: <Shape>");
}

/// Phase 2 Slice 6, criterion 8 (M6, R5): a self-tail-recursive word defined
/// at the REPL gets the same self-tail-call -> loop transform as the native
/// path (`lower_word` is shared, so the current-word name reaches the REPL
/// lowering with no REPL-specific plumbing) and completes in constant stack
/// over N >= 1_000_000, the depth at which un-transformed recursion would
/// overflow the host stack.
#[ignore = "same unimplemented REPL trait/impl checking as \
    `sign_definable_and_callable_in_repl` -- see that test's #[ignore] note."]
#[test]
fn self_tail_recursive_word_completes_in_constant_stack_in_repl() {
    let cmp = common::repl_core_import("cmp", "eq");
    let boolean = common::repl_core_import("bool", "if");
    let out = run_session(&[
        &cmp,
        &boolean,
        ": sum-to ( i64 i64 -- i64 ) | acc n | n 0 eq ~[ acc ] ~[ acc n add n 1 sub sum-to ] if ;",
        "0 1000000 sum-to .",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "imported cmp",
            "imported bool",
            "defined sum-to",
            "500000500000",
            "stack: (empty)"
        ]
    );
}

/// Phase 2 Slice 7, criterion 7 (D7): the `examples/vm.sth` bytecode-VM
/// dogfood, defined and run entirely as REPL session lines (each definition
/// flattened onto one line, since the REPL reads one line at a time). The
/// self-tail-recursive `run` gets the same self-tail-call -> loop transform
/// through the `dlopen` path with no REPL-specific plumbing (established by
/// Slice 6 criterion 8), and the session runs the same N = 100_000 program as
/// the native golden, so "the same result" is literal.
#[ignore = "same unimplemented REPL trait/impl checking as \
    `sign_definable_and_callable_in_repl` -- see that test's #[ignore] note."]
#[test]
fn vm_dogfood_runs_in_repl() {
    let cmp = common::repl_core_import("cmp", "eq");
    let boolean = common::repl_core_import("bool", "if");
    let out = run_session(&[
        &cmp,
        &boolean,
        "type: Op | Push v i64 | Add | Sub | Mul | Load addr usize | Store addr usize | Jz target usize | Jmp target usize | Halt ;",
        "type: Vm prog [Op 13] pc usize stack [i64 8] sp usize mem [i64 4] ;",
        "type: Fetched vm Vm op Op ;",
        "type: VmPop vm Vm val i64 ;",
        ": vm-push ( Vm i64 -- Vm ) | vm x | vm &sp @ swap drop | i | &!vm &!stack i &!> x ! vm &sp @ swap drop 1 add | newsp | vm &!sp newsp ! ;",
        ": vm-pop ( Vm -- VmPop ) | vm | vm &sp @ swap drop 1 sub | i | &vm &stack i &> @ | x | vm &!sp i ! x VmPop ;",
        ": bump-pc ( Vm -- Vm ) &pc @ 1 add | newpc | &!pc newpc ! ;",
        ": fetch ( Vm -- Fetched ) | vm | vm &pc @ swap drop | i | &vm &prog i &> @ | op | vm op Fetched ;",
        ": run ( Vm Op -- i64 ) ~[ ( Push ) Push> | vm v | vm v vm-push bump-pc fetch Fetched> run ] ~[ ( Add ) drop | vm | vm vm-pop VmPop> | b | vm-pop VmPop> b add vm-push bump-pc fetch Fetched> run ] ~[ ( Sub ) drop | vm | vm vm-pop VmPop> | b | vm-pop VmPop> b sub vm-push bump-pc fetch Fetched> run ] ~[ ( Mul ) drop | vm | vm vm-pop VmPop> | b | vm-pop VmPop> b mul vm-push bump-pc fetch Fetched> run ] ~[ ( Load ) Load> | vm addr | &vm &mem addr &> @ | x | vm x vm-push bump-pc fetch Fetched> run ] ~[ ( Store ) Store> | vm addr | vm vm-pop VmPop> | v x | &!v &!mem addr &!> x ! v bump-pc fetch Fetched> run ] ~[ ( Jz ) Jz> | vm target | vm vm-pop VmPop> 0 eq ~[ &!pc target ! ] ~[ bump-pc ] if fetch Fetched> run ] ~[ ( Jmp ) Jmp> | vm target | vm &!pc target ! fetch Fetched> run ] ~[ ( Halt ) drop | vm | vm vm-pop VmPop> swap drop ] Op? ;",
        ": build ( -- [Op 13] ) Halt 13 fill | prog | &!prog 0 >usize &!> 0 >usize Load ! &!prog 1 >usize &!> 11 >usize Jz ! &!prog 2 >usize &!> 1 >usize Load ! &!prog 3 >usize &!> 0 >usize Load ! &!prog 4 >usize &!> Add ! &!prog 5 >usize &!> 1 >usize Store ! &!prog 6 >usize &!> 0 >usize Load ! &!prog 7 >usize &!> 1 Push ! &!prog 8 >usize &!> Sub ! &!prog 9 >usize &!> 0 >usize Store ! &!prog 10 >usize &!> 0 >usize Jmp ! &!prog 11 >usize &!> 1 >usize Load ! prog ;",
        "build 0 >usize 0 8 fill 0 >usize 0 4 fill | mem | &!mem 0 >usize &!> 100000 ! mem Vm fetch Fetched> run .",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "imported cmp",
            "imported bool",
            "defined type Op",
            "defined type Vm",
            "defined type Fetched",
            "defined type VmPop",
            "defined vm-push",
            "defined vm-pop",
            "defined bump-pc",
            "defined fetch",
            "defined run",
            "defined build",
            "5000050000",
            "stack: (empty)",
        ]
    );
}

// Phase 3 Slice 1, criterion 14: the REPL session is the interactive program's
// "main" word and `:quit` is the end of its scope, so residual linear values are
// disposed there (top first) rather than leaked. A live session can never prove
// "you forgot to dispose this" at compile time, since the next line might
// consume it, but exactly-once still holds.

#[test]
fn repl_quit_disposes_residual_linear() {
    let out = run_session(&[SPY_TYPE_LINE, SPY_DROP_LINE, "7 Spy", "8 Spy", ":quit"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "stack: <Spy>",
            "stack: <Spy> <Spy>",
            "drop 8",
            "drop 7",
        ],
        "residual Spy values should be disposed at `:quit`, top of stack first"
    );
}

#[test]
fn repl_within_one_line_create_and_drop_prints_once() {
    let out = run_session(&[SPY_TYPE_LINE, SPY_DROP_LINE, "7 Spy drop", ":quit"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "drop 7",
            "stack: (empty)",
        ],
        "a Spy created and dropped within one line should print exactly once"
    );
}

#[test]
fn repl_explicit_drop_not_redisposed_at_quit() {
    let out = run_session(&[SPY_TYPE_LINE, SPY_DROP_LINE, "7 Spy", "drop", ":quit"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "stack: <Spy>",
            "drop 7",
            "stack: (empty)",
        ],
        "a Spy dropped on an earlier line prints once, not again at `:quit`"
    );
}

#[test]
fn repl_word_definition_keeps_strict_linear_rule() {
    // `:quit`'s residual disposal is a REPL-session-only relaxation; a word
    // DEFINITION typed at the REPL is still checked by the ordinary strict
    // rule (forgetting a linear value is a compile error, not an auto-drop),
    // and the bad definition reports and rolls back rather than killing the
    // session.
    let out = run_session(&[
        SPY_TYPE_LINE,
        SPY_DROP_LINE,
        ": bad ( -- ) 7 Spy ;",
        "bad",
        "1 .",
        ":quit",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "defined type Spy");
    assert_eq!(lines[1], "defined drop for Spy");
    assert!(
        lines[2].contains("linear value left on the stack") && lines[2].contains("`bad`"),
        "expected the surplus-linear diagnostic naming `bad`: {}",
        lines[2]
    );
    assert_eq!(
        &lines[3..5],
        [
            "  body leaves a `Spy` beyond the 0 declared output(s): a linear value must be consumed exactly once, so `drop` it or return it",
            "  note: declared ( -- )",
        ],
        "the session should survive the bad definition and keep processing later lines: {out}"
    );
    // The rejected definition must not have half-landed: calling `bad` next
    // is an unknown word, not a call into a partially-registered one.
    assert!(
        lines[5].contains("unknown word") && lines[5].contains("bad"),
        "expected `bad` to be unregistered after its rejected definition: {}",
        lines[5]
    );
    assert_eq!(&lines[6..], ["1", "stack: (empty)"]);
}

// Phase 2: the synthesized struct destructor (`sooth_struct_drop_N`) must be
// emitted into every REPL module that can reach a `drop` on that struct type
// (a bare line's module, a `: word ;` definition's module, and the synthesized
// `:quit` disposal), not only the build path's single shared module.

#[test]
fn repl_bare_line_drops_linear_struct() {
    let out = run_session(&[
        SPY_TYPE_LINE,
        SPY_DROP_LINE,
        "type: Pair a Spy b Spy ;",
        "1 Spy 2 Spy Pair",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "defined type Pair",
            "stack: <Pair>",
            "drop 1",
            "drop 2",
            "stack: (empty)",
        ]
    );
}

#[test]
fn repl_word_definition_drops_linear_struct() {
    let out = run_session(&[
        SPY_TYPE_LINE,
        SPY_DROP_LINE,
        "type: Pair a Spy b Spy ;",
        ": mk ( -- ) 1 Spy 2 Spy Pair drop ;",
        "mk",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "defined type Pair",
            "defined mk",
            "drop 1",
            "drop 2",
            "stack: (empty)",
        ]
    );
}

#[test]
fn repl_quit_disposes_residual_linear_struct() {
    let out = run_session(&[
        SPY_TYPE_LINE,
        SPY_DROP_LINE,
        "type: Pair a Spy b Spy ;",
        "1 Spy 2 Spy Pair",
        ":quit",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "defined type Pair",
            "stack: <Pair>",
            "drop 1",
            "drop 2",
        ],
        "a residual linear struct should be disposed at `:quit`, field-order drop"
    );
}

// Phase 4: the synthesized enum destructor (`sooth_enum_drop_N`) must be
// emitted into every REPL module that can reach a `drop` on that enum type,
// exactly like the struct case above; a missing symbol here is a `dlopen`
// failure, not a compile error, so it needs its own coverage.

#[test]
fn repl_word_definition_drops_linear_enum() {
    let out = run_session(&[
        SPY_TYPE_LINE,
        SPY_DROP_LINE,
        "type: Item | Empty | Full v Spy ;",
        ": mk ( -- ) 1 Spy Full drop ;",
        "mk",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "defined type Item",
            "defined mk",
            "drop 1",
            "stack: (empty)"
        ]
    );
}

#[test]
fn repl_quit_disposes_residual_linear_enum() {
    let out = run_session(&[
        SPY_TYPE_LINE,
        SPY_DROP_LINE,
        "type: Item | Empty | Full v Spy ;",
        "1 Spy Full",
        ":quit",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "defined type Item",
            "stack: <Item>",
            "drop 1",
        ],
        "a residual linear enum should be disposed at `:quit`, tag-dispatched"
    );
}

// Phase 3 Slice 2 (criterion 15): a `^T` cell left on the residual REPL stack
// is freed at `:quit` through the same `dispose_residual` path as a Spy and
// the struct/enum cases above, needing no production change beyond Phase 1's
// session-persistent cell registry. The trace is gated on, so the transcript
// is asserted exactly, `alloc` at construction then `free` at `:quit`.

#[test]
fn repl_quit_frees_residual_owned() {
    let out = run_session_traced(&["5 ^", ":quit"], true);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["alloc 8", "stack: <^i64>", "free 8"],
        "a residual owned cell should be freed at `:quit`"
    );
}

// Phase 3 Slice 3, phase 5 (criterion 18): a residual directly self-recursive
// value at `:quit` goes through `dispose_residual`'s ordinary `emit_drop`
// path, which is exactly the synthesized destructor call site every other
// `drop` uses — so this exercises the fused loop (R11) from the REPL rather
// than only from a compiled `main`, needing no production change beyond what
// phase 4 already built.
#[test]
fn repl_quit_frees_residual_recursive_value() {
    let out = run_session_traced(
        &[
            SPY_TYPE_LINE,
            SPY_DROP_LINE,
            "type: List | Nil | Cons tag Spy next ^List ;",
            "1 Spy Nil ^ Cons",
            "2 Spy swap ^ Cons",
            ":quit",
        ],
        true,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "defined type List",
            "alloc 24",
            "stack: <List>",
            "alloc 24",
            "stack: <List>",
            "drop 2",
            "free 24",
            "drop 1",
            "free 24",
        ],
        "a residual recursive list should dispose node-by-node through the \
         fused loop, top node's tag first (pre-order)"
    );
}

// Phase 3 upgrades criterion F: a clean-bodied polymorphic REPL definition
// is now correctly *supported*, not rejected (recon-1's silent miscompile
// stays gone either way, but the Phase 1 blanket rejection is no longer what
// a valid definition like `id` sees).
#[test]
fn polymorphic_repl_definition_with_clean_body_is_accepted_not_rejected() {
    let out = run_session(&[": id ( 'T -- 'T ) ;"]);
    assert_eq!(out, "defined id\n");
}

// Phase 3 upgrades `twice`'s half of criterion F: its rejection is now the
// real X1 diagnostic from `check_poly_body` (naming `'T` and the missing
// `Copy` bound an unbounded `dup` needs), not the Phase 1 blanket
// "polymorphic ... REPL" wording, and not the recon-1 `( -- )` mismatch.
#[test]
fn polymorphic_repl_definition_with_ill_typed_body_is_the_real_x1_not_the_old_blanket_rejection() {
    let out = run_session(&[": twice ( 'T -- 'T 'T ) dup ;"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "unexpected output:\n{out}");
    assert!(
        lines[0].contains("cannot `dup` the type variable `'T`") && lines[0].contains("`twice`"),
        "expected the X1 dup-of-unbounded-variable diagnostic: {}",
        lines[0]
    );
    assert!(
        lines[1].contains("'T") && lines[1].contains("Copy"),
        "expected the missing-Copy-bound reason: {}",
        lines[1]
    );
    assert!(
        !out.contains("REPL"),
        "must not be the Phase 1 blanket polymorphic-REPL rejection: {out}"
    );
    assert!(
        !out.contains("declared ( -- )"),
        "must not be the recon-1 zero-arity mismatch: {out}"
    );
}

// Criterion 1 (trace A): defining `id` once and instantiating it at two
// different concrete types on later lines prints each instantiation's value.
#[test]
fn polymorphic_repl_word_instantiates_at_two_different_types_across_lines() {
    let out = run_session(&[": id ( 'T -- 'T ) ;", "5 id .", "\"hi\" id ."]);
    // `.` on a `str` prints via `%.*s` with no trailing newline (unlike every
    // other printable type, see `backend/qbe.rs`'s `$strfmt`), so "hi" runs
    // directly into the following "stack: (empty)" line; this is that
    // formatter's real behaviour, not a test bug.
    assert_eq!(
        out, "defined id\n5\nstack: (empty)\nhistack: (empty)\n",
        "unexpected output:\n{out}"
    );
}

// Criterion 2 (trace B): a second same-type instantiation prints its value
// without recompiling anything (the dedup itself is pinned by the
// `exported_insts`-size unit assertion in `src/repl.rs`).
#[test]
fn polymorphic_repl_word_instantiated_twice_at_one_type_prints_both_values() {
    let out = run_session(&[": id ( 'T -- 'T ) ;", "5 id .", "7 id ."]);
    assert_eq!(
        out, "defined id\n5\nstack: (empty)\n7\nstack: (empty)\n",
        "unexpected output:\n{out}"
    );
}

// D3 (both halves of the frozen callee-binding): a poly word's body calls a
// concrete word that is then redefined at a *different arity/return type*
// before the poly word is instantiated. The instantiation must resolve the
// callee against the defining-line snapshot -- both its frozen symbol *and*
// its frozen arity -- so `noise`'s gen0 `( -- )` no-op body runs, leaving `p`
// the identity: `5 p .` prints `5`. Before arity was frozen, the frozen
// symbol was called under the redefined `( i64 -- i64 )` ABI, reading an
// uninitialized argument slot and printing garbage.
#[test]
fn poly_instantiation_freezes_callee_arity_across_a_differing_redefinition() {
    let out = run_session(&[
        ": noise ( -- ) ;",
        ": p ( 'T -- 'T ) noise ;",
        ": noise ( i64 -- i64 ) | n | n 100 add ;",
        "5 p .",
    ]);
    assert_eq!(
        out, "defined noise\ndefined p\ndefined noise\n5\nstack: (empty)\n",
        "unexpected output:\n{out}"
    );
}

// D3 same-arity control: when the redefined callee keeps its arity/return
// type, the frozen-symbol binding alone already pins the old body. `noise`
// stays `( -- i64 )`; `p`'s instantiation binds `noise`@gen0 (value 42), so
// a later `noise` redefinition to 99 cannot change `p`'s meaning: `5 p .`
// prints `42`, the frozen gen0 value, not `99`. This is the value-witnessed
// counterpart to the arity case above, and guards the frozen-symbol property
// independently of the arity fix.
#[test]
fn poly_instantiation_freezes_callee_value_across_a_same_arity_redefinition() {
    let out = run_session(&[
        ": noise ( -- i64 ) 42 ;",
        ": p ( 'T -- i64 ) drop noise ;",
        ": noise ( -- i64 ) 99 ;",
        "5 p .",
    ]);
    assert_eq!(
        out, "defined noise\ndefined p\ndefined noise\n42\nstack: (empty)\n",
        "unexpected output:\n{out}"
    );
}

// X2: instantiating a `'T: Copy` REPL word at a linear concrete type on a
// later line is the native call-site error (`Ctx::Line` phrasing), naming
// the variable, the callee, and the linear type.
#[test]
fn polymorphic_repl_word_instantiated_at_linear_type_without_copy_bound_is_x2() {
    let out = run_session(&[
        ": id ( 'T: Copy -- 'T ) ;",
        SPY_TYPE_LINE,
        SPY_DROP_LINE,
        "0 Spy id drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 4, "unexpected output:\n{out}");
    assert_eq!(
        &lines[..3],
        ["defined id", "defined type Spy", "defined drop for Spy"]
    );
    let err = lines[3];
    assert!(
        err.contains("'T") && err.contains("id") && err.contains("Spy") && err.contains("Copy"),
        "expected an X2 diagnostic naming 'T, `id`, and `Spy`'s Copy bound: {err}"
    );
}

// X3: a polymorphic REPL definition resolving to two or more concrete
// outputs is a clean located deferral, not a silent single-output
// truncation, never `defined pair`.
#[test]
fn polymorphic_repl_definition_resolving_to_two_outputs_is_a_located_x3() {
    let out = run_session(&[": pair ( 'T: Copy -- 'T 'T ) dup ;"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "unexpected output:\n{out}");
    assert!(
        lines[0].contains("`pair`") && lines[0].contains("2 outputs"),
        "expected the X3 multi-output deferral naming `pair`: {}",
        lines[0]
    );
    assert!(
        lines[1].contains("return bundle"),
        "expected the deferred-return-bundle reason: {}",
        lines[1]
    );
    assert_ne!(
        out, "defined pair\n",
        "must not silently truncate to one output"
    );
}

// Criterion 3 (trace C, R8): redefining a polymorphic word follows the
// ordinary-word generation rule -- an earlier line's compiled call keeps the
// old generation's body (frozen binding, D3/D4) while a new call site binds
// the new generation. Single-output throughout: `id` starts unbounded
// (`( 'T -- 'T )`, gen0), so it instantiates even at the linear `Spy`; `g`
// binds `id`@`Spy` at gen0 and stays observable (`drop 7`). Redefining `id`
// to add a `Copy` bound (gen1) leaves `g`'s compiled gen0 call untouched
// (still `drop 7`) while a *new* `7 Spy id drop` line now fails the `Copy`
// bound (X2). That is the generation-freezing property, witnessed without a
// return bundle.
#[test]
fn redefined_polymorphic_word_freezes_earlier_call_while_new_call_rebinds() {
    let out = run_session(&[
        SPY_TYPE_LINE,
        SPY_DROP_LINE,
        ": id ( 'T -- 'T ) ;",
        ": g ( -- ) 7 Spy id drop ;",
        "g",
        ": id ( 'T: Copy -- 'T ) ;",
        "g",
        "7 Spy id drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 10, "unexpected output:\n{out}");
    assert_eq!(
        &lines[..9],
        [
            "defined type Spy",
            "defined drop for Spy",
            "defined id",
            "defined g",
            // `g` at gen0: id@Spy is the identity, `drop` prints once.
            "drop 7",
            "stack: (empty)",
            // id redefined to gen1 (adds the Copy bound).
            "defined id",
            // `g` still runs its frozen gen0 id@Spy body: unchanged.
            "drop 7",
            "stack: (empty)",
        ],
        "an earlier line's compiled call must stay frozen to gen0 across the redefinition:\n{out}"
    );
    // The new bare line instantiates id at gen1, whose `Copy` bound rejects
    // the linear `Spy` (X2's `Ctx::Line` phrasing), naming the variable, the
    // callee, and `Spy`.
    let err = lines[9];
    assert!(
        err.contains("'T") && err.contains("id") && err.contains("Spy") && err.contains("Copy"),
        "expected the gen1 Copy-bound rejection naming 'T, `id`, and `Spy`: {err}"
    );
}

// R8 (shared per-name counter, both directions): a name toggling
// ordinary -> poly -> ordinary must not remint a resident symbol. The first
// ordinary `id` exports `id__gen0`; defining `id` as poly evicts it from the
// ordinary env; redefining `id` as ordinary again must take gen2 (past the
// poly entry), not reset to gen0 and collide with the first body under
// `RTLD_GLOBAL` first-loaded-wins. Witnessed by the last call observing the
// *new* body (`add 2` -> 7), not the shadowed first (`add 1` -> 6).
#[test]
fn ordinary_word_redefined_across_a_poly_definition_does_not_remint_the_old_symbol() {
    let out = run_session(&[
        ": id ( i64 -- i64 ) | n | n 1 add ;",
        "5 id",
        ": id ( 'T -- 'T ) ;",
        ": id ( i64 -- i64 ) | n | n 2 add ;",
        "5 id",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined id",
            "stack: 6",
            "defined id",
            "defined id",
            // The `6` from line 2 persists on the session stack; the final
            // `5 id` must run its own generation's body (5 + 2 = 7) on top,
            // not the first `id__gen0` shadowing it (which would give 6).
            "stack: 6 7",
        ],
        "the redefined ordinary `id` must not collide with the pre-poly generation:\n{out}"
    );
}

// Criterion 4: the consolidated ROADMAP exit session for this slice, one
// golden covering the whole sequence in the spec's own words: define `id`
// once, instantiate it at two different concrete types on later lines,
// instantiate it twice at one type without recompiling (dedup), redefine
// it, and see the new body take effect on the next call while an earlier
// line's call keeps the old one. The recon-1 silent miscompile (the bogus
// `note: declared ( -- )` mismatch, or a silent `defined id` that never
// checked the body) is gone throughout: every line here is either a real
// printed value or, for the redefinition witness, a real X2 diagnostic
// (single-output throughout, D5's flagged deviation from the brief's
// 2-output trace-C witness, R7's multi-output carve-out).
#[test]
fn consolidated_exit_session_covers_define_instantiate_dedup_and_redefine() {
    let out = run_session(&[
        SPY_TYPE_LINE,
        SPY_DROP_LINE,
        ": id ( 'T -- 'T ) ;",
        // Instantiate at two different concrete types (trace A).
        "5 id .",
        "\"hi\" id .",
        // A second same-type instantiation recompiles nothing (trace B).
        "7 id .",
        // A defined word's own body calls the retained poly word, binding
        // `id`@`Spy` at gen0 (R5's word-def check path, R4's frozen
        // resolver snapshot).
        ": g ( -- ) 7 Spy id drop ;",
        "g",
        // Redefine `id`, adding a `Copy` bound: gen1. `g`'s already-compiled
        // call stays frozen to gen0 (D3/D4); only a *new* instantiation
        // sees gen1.
        ": id ( 'T: Copy -- 'T ) ;",
        "g",
        "7 Spy id drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 15, "unexpected output:\n{out}");
    assert_eq!(
        &lines[..14],
        [
            "defined type Spy",
            "defined drop for Spy",
            "defined id",
            "5",
            "stack: (empty)",
            // `.` on a `str` prints with no trailing newline, so it runs
            // directly into this line's own stack printout.
            "histack: (empty)",
            // Dedup: the second `i64` instantiation just runs the symbol
            // exported by the first, no recompile.
            "7",
            "stack: (empty)",
            "defined g",
            // `g` at gen0: id@Spy is the identity, `drop` prints once.
            "drop 7",
            "stack: (empty)",
            "defined id",
            // `g` still runs its frozen gen0 id@Spy body: unchanged.
            "drop 7",
            "stack: (empty)",
        ],
        "unexpected output:\n{out}"
    );
    // The new bare line instantiates `id` at gen1, whose `Copy` bound
    // rejects the linear `Spy` -- the new body taking effect while the
    // earlier `g` call kept the old one.
    let err = lines[14];
    assert!(
        err.contains("'T") && err.contains("id") && err.contains("Spy") && err.contains("Copy"),
        "expected the gen1 Copy-bound rejection naming 'T, `id`, and `Spy`: {err}"
    );
    assert!(
        !out.contains("declared ( -- )"),
        "must not be the recon-1 zero-arity mismatch: {out}"
    );
}
