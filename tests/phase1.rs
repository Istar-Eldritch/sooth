//! Phase 1 golden session tests: spawn the `repl` binary, pipe a scripted
//! stdin session, and assert on stdout. Each test is one exit criterion.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run a scripted REPL session (one input line per element of `lines`) and
/// return the whole captured stdout.
fn run_session(lines: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
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
fn define_then_call_across_lines() {
    let out = run_session(&[": sq ( i64 -- i64 ) | n | n n * ;", "5 sq"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["defined sq", "stack: 25"]);
}

#[test]
fn stack_persists_across_lines() {
    let out = run_session(&[": sq ( i64 -- i64 ) | n | n n * ;", "5", "sq", "1 +"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["defined sq", "stack: 5", "stack: 25", "stack: 26"]
    );
}

#[test]
fn redefinition_takes_effect_for_later_lines() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) | n | n n * ;",
        "3 sq",
        ": sq ( i64 -- i64 ) | n | n n n * * ;",
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
    let out = run_session(&["5", "unknown-word", "1 +"]);
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
    let out = run_session(&["5", "true 1 +", "1 +"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "stack: 5");
    assert!(
        lines[1].contains("`i64`") && lines[1].contains("`bool`"),
        "expected a type-mismatch diagnostic: {}",
        lines[1]
    );
    assert_eq!(lines[2], "stack: 6");
}

#[test]
fn failed_redefinition_keeps_old_generation_resident() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) | n | n n * ;",
        "3 sq",
        ": sq ( i64 -- i64 ) dup dup * ;",
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
    // original generation (`n n *`), and the stack from the first `3 sq` is
    // untouched, so the second `3 sq` appends its own 9.
    assert_eq!(lines[5], "stack: 9 9");
}

#[test]
fn sign_definable_and_callable_in_repl() {
    let out = run_session(&[
        ": sign ( i64 -- i64 ) 0 > if 1 else 0 end ;",
        "-7 sign",
        "7 sign",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["defined sign", "stack: 0", "stack: 0 1"]);
}

#[test]
fn bool_residual_displays_as_true_or_false() {
    // Matches `.`'s print semantics: `true`/`false`, not the raw 0/1.
    let out = run_session(&["true", "false"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["stack: true", "stack: true false"]);
}

#[test]
fn calculator_session_dogfood() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) | n | n n * ;",
        ": neg ( i64 -- i64 ) 0 swap - ;",
        "3 sq",
        "neg",
        "10 +",
        "2 *",
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
    let out = run_session(&["200 >u8", "100 >u8 +", ">i64 ."]);
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
    let out = run_session(&["1.5 2.0 +", "1.0 +", "."]);
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
    let out = run_session(&["type: Vec2 x i64 y i64 ;", "5 6 Vec2", "Vec2>x ."]);
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
    let out = run_session(&["type: Vec2 x i64 y i64 ;", "5 6 Vec2 99", "drop Vec2>y ."]);
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
        "dup Pair>a . Pair>b .",
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
        "5 6 Vec2 Vec2>y .",
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
/// two calls exercising the nested `Segment>`/`sub`/`len2` span and the
/// `shift-x` functional setter).
#[test]
fn vectors_dogfood_runs_in_repl() {
    let out = run_session(&[
        "type: Vec2 x i64 y i64 ;",
        "type: Segment from Vec2 to Vec2 ;",
        ": sub ( Vec2 Vec2 -- Vec2 ) | a b | a Vec2>x b Vec2>x - a Vec2>y b Vec2>y - Vec2 ;",
        ": len2 ( Vec2 -- i64 ) | v | v Vec2>x v Vec2>x * v Vec2>y v Vec2>y * + ;",
        ": span ( Segment -- Vec2 ) Segment> swap sub ;",
        ": shift-x ( Vec2 i64 -- Vec2 ) | v dx | v v Vec2>x dx + Vec2<x ;",
        "0 0 Vec2 3 4 Vec2 Segment span len2 .",
        "5 6 Vec2 1 shift-x Vec2>x .",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Vec2",
            "defined type Segment",
            "defined sub",
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
    let out = run_session(&["5 .", "2 3 + ."]);
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

/// Criterion 7: an enum is declared on one line, then a clause-style word is
/// defined over it on a *later* line (R18's D8 variant-set seeding from
/// `Session.enums`, since the parser pre-pass alone only scans the current
/// unit). A value constructs and displays `<Shape>`, then a further line
/// eliminates it through the clause word.
#[test]
fn enum_declared_then_clause_word_defined_and_eliminated_on_later_lines() {
    let out = run_session(&[
        "type: Shape | Circle r f64 | Rect w f64 h f64 ;",
        ": area ( Shape -- f64 ) | Circle dup * 3.14159 * | Rect | w h | w h * ;",
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
/// input isn't the point of this golden; exercising every clause arm from the
/// dogfood file is), and all four `main` operations run, hitting the exact
/// native golden's output (`12.5664 / 12 / 5 / 7`) through the REPL path.
#[test]
fn shapes_dogfood_runs_full_program_in_repl() {
    let out = run_session(&[
        "type: Shape | Circle r f64 | Rect w f64 h f64 ;",
        "type: MaybeInt | None | Some v i64 ;",
        ": area ( Shape -- f64 ) | Circle dup * 3.14159 * | Rect | w h | w h * ;",
        ": unwrap-or ( i64 MaybeInt -- i64 ) | None | Some swap drop ;",
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

/// Criterion 8: the `examples/stack.sth` dogfood — array-as-struct-field, a
/// runtime `usize` cursor (the trap path), non-consuming `get`, functional
/// `set`, and `len` — declared and run entirely across REPL lines (`type:`
/// with an array field, then `fill`/`get`/`set`/`len` at REPL scope,
/// mirroring the Slice 4 REPL-scope seeding).
#[test]
fn stack_dogfood_runs_in_repl() {
    let out = run_session(&[
        "type: Stack items [i64 16] top usize ;",
        "type: Popped rest Stack item i64 ;",
        ": empty ( -- Stack ) 0 16 fill 0 >usize Stack ;",
        ": push ( Stack i64 -- Stack ) | s x | s s Stack>items s Stack>top x set Stack<items s Stack>top 1 + Stack<top ;",
        ": pop ( Stack -- Popped ) | s | s s Stack>top 1 - Stack<top s Stack>items s Stack>top 1 - get swap drop Popped ;",
        ": peek ( Stack -- Popped ) | s | s s Stack>items s Stack>top 1 - get swap drop Popped ;",
        "empty 1 push 2 push 3 push",
        "peek Popped> .",
        "pop Popped> .",
        "pop Popped> .",
        "peek Popped> .",
        "Stack>items len .",
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
#[test]
fn self_tail_recursive_word_completes_in_constant_stack_in_repl() {
    let out = run_session(&[
        ": sum-to ( i64 i64 -- i64 ) | acc n | n 0 = if acc else acc n + n 1 - sum-to end ;",
        "0 1000000 sum-to .",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["defined sum-to", "500000500000", "stack: (empty)"]
    );
}

/// Phase 2 Slice 7, criterion 7 (D7): the `examples/vm.sth` bytecode-VM
/// dogfood, defined and run entirely as REPL session lines (each definition
/// flattened onto one line, since the REPL reads one line at a time). The
/// self-tail-recursive `run` gets the same self-tail-call -> loop transform
/// through the `dlopen` path with no REPL-specific plumbing (established by
/// Slice 6 criterion 8), and the session runs the same N = 100_000 program as
/// the native golden, so "the same result" is literal.
#[test]
fn vm_dogfood_runs_in_repl() {
    let out = run_session(&[
        "type: Op | Push v i64 | Add | Sub | Mul | Load addr usize | Store addr usize | Jz target usize | Jmp target usize | Halt ;",
        "type: Vm prog [Op 13] pc usize stack [i64 8] sp usize mem [i64 4] ;",
        "type: Fetched vm Vm op Op ;",
        "type: VmPop vm Vm val i64 ;",
        ": vm-push ( Vm i64 -- Vm ) | vm x | vm vm Vm>stack vm Vm>sp x set Vm<stack vm Vm>sp 1 + Vm<sp ;",
        ": vm-pop ( Vm -- VmPop ) | vm | vm vm Vm>sp 1 - Vm<sp vm Vm>stack vm Vm>sp 1 - get swap drop VmPop ;",
        ": bump-pc ( Vm -- Vm ) dup Vm>pc 1 + Vm<pc ;",
        ": fetch ( Vm -- Fetched ) | vm | vm vm Vm>prog vm Vm>pc get swap drop Fetched ;",
        ": run ( Vm Op -- i64 ) | Push | vm v | vm v vm-push bump-pc fetch Fetched> run | Add | vm | vm vm-pop VmPop> swap vm-pop VmPop> rot + vm-push bump-pc fetch Fetched> run | Sub | vm | vm vm-pop VmPop> swap vm-pop VmPop> rot - vm-push bump-pc fetch Fetched> run | Mul | vm | vm vm-pop VmPop> swap vm-pop VmPop> rot * vm-push bump-pc fetch Fetched> run | Load | vm addr | vm vm Vm>mem addr get swap drop vm-push bump-pc fetch Fetched> run | Store | vm addr | vm vm-pop VmPop> over Vm>mem addr rot set Vm<mem bump-pc fetch Fetched> run | Jz | vm target | vm vm-pop VmPop> 0 = if target Vm<pc else bump-pc end fetch Fetched> run | Jmp | vm target | vm target Vm<pc fetch Fetched> run | Halt | vm | vm vm-pop VmPop> swap drop ;",
        ": build ( -- [Op 13] ) Halt 13 fill 0 >usize 0 >usize Load set 1 >usize 11 >usize Jz set 2 >usize 1 >usize Load set 3 >usize 0 >usize Load set 4 >usize Add set 5 >usize 1 >usize Store set 6 >usize 0 >usize Load set 7 >usize 1 Push set 8 >usize Sub set 9 >usize 0 >usize Store set 10 >usize 0 >usize Jmp set 11 >usize 1 >usize Load set ;",
        "build 0 >usize 0 8 fill 0 >usize 0 4 fill 0 >usize 100000 set Vm fetch Fetched> run .",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
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
