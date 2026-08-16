# Phase 2 Slice 2 brief — the integer tower and explicit conversions

Input for spec-writer. Phase 2 (typed core) is an epic; this is its second slice,
building directly on Slice 1 (the typed-core spine, now on `main`). Read alongside
[../DESIGN.md](../DESIGN.md), [../ROADMAP.md](../ROADMAP.md), and
[../CLAUDE.md](../CLAUDE.md). Everything here is scoped to Slice 2 only, on top of the
Phase 0/1/Slice-1 compiler already on `main` (lexer/parser/checker/IR/QBE-emit/driver/REPL,
with a type-carrying checker over `i64` and `bool`).

## Why this slice, and why it is worth doing alone

Slice 1 proved the spine: each stack slot carries a `Type`, and the checker unifies types
(not just depth) through bodies and at branch joins, with exactly two types (`i64`, `bool`)
so a mismatch is possible. Slice 2 widens the type set along one axis, the **integer
tower**, and introduces the machinery that a real tower forces and Slice 1 could dodge:

1. **Width.** A type narrower than 64 bits (`i8`/`i16`/`i32`, and the unsigned widths) means
   codegen must track a register class (`w` vs `l`) and keep sub-word values canonical.
2. **Signedness.** `u8`..`u64` mean widening (sign- vs zero-extend) and comparison (`cslt`
   vs `cult`) now depend on the operand type, not just its width.
3. **Explicit conversion.** With more than one integer type, moving between them must be a
   deliberate, visible operation, not an implicit promotion.

These three are the load-bearing new mechanisms. Floats, `i128`/`u128`, bitwise operators,
and the `*/` widening primitive are separate axes and are all out of scope (see below).

## Type set for Slice 2

The full fixed-width integer tower plus the existing `bool`:

- Signed: `i8`, `i16`, `i32`, `i64`
- Unsigned: `u8`, `u16`, `u32`, `u64`
- `bool` (unchanged from Slice 1)

Frontend `Type` grows from the flat `{ I64, Bool }` to carry width and signedness for the
integer case, e.g. `Type::Int { bits: u8, signed: bool }` plus `Type::Bool`. `bits` is one
of 8/16/32/64. Display renders `i8`..`i64` / `u8`..`u64` / `bool`. The eight integer types
are table-generated, not eight hand-written enum variants, so breadth stays cheap.

## Decisions locked for this slice

1. **Full integer set in one slice.** All eight widths land now (not a minimal proving
   subset). The mechanism is width + signedness + conversion; once it holds, the eight
   types are table-fill, so ship the breadth.
2. **Literals stay `i64`; you convert down explicitly.** An integer literal is `i64`. There
   is **no** type inference and no context-directed literal typing (Slice 1 deliberately
   avoided an inference framework; keep it out). To get a narrower or unsigned literal you
   write an explicit conversion, e.g. `255 >u8`.
3. **Arithmetic is homogeneous per type, no implicit promotion.** `+ - * mod` require both
   operands to be the **same integer type** and produce that type. `i32 i64 +` is a sharp
   type error. There is no automatic widening; convert first.
4. **Comparisons are homogeneous, produce `bool`, and pick signed vs unsigned codegen by
   operand type.** `< >` emit a signed compare (`cslt`/`csgt`) for signed operands and an
   unsigned compare (`cult`/`cugt`) for unsigned operands. `=` is signedness-agnostic. Both
   operands must be the same integer type (mixed-width comparison is an error).
5. **Conversions are explicit, target-only words: `>i8`, `>i16`, `>i32`, `>i64`, `>u8`,
   `>u16`, `>u32`, `>u64`.** Each is `( <any integer type> -- <target> )`: it pops one
   integer value of any integer type and pushes the target type. Semantics:
   - **Widening** sign-extends when the **source** type is signed, zero-extends when the
     source is unsigned.
   - **Narrowing** truncates (wraps, well-defined), keeping the low `bits` of the value.
   - Same-width sign reinterpretation (`i32 >u32`) is a no-op on the bits, a type relabel.
   - The source must be an integer type; `>i32` applied to a `bool` is an error. There are
     no `>bool` conversions (bool is not in the numeric tower).
   - **Checked and saturating conversions are out of scope** (deferred). Narrowing is
     silently truncating for now, which is the one deliberately-unsharp edge this slice
     ships; call it out in the docs.
6. **`.` (print) stays `( i64 -- )`.** No per-width or unsigned printing. You widen to `i64`
   to print (`>i64 .`). This is consistent with the convert-explicitly stance and, as a
   bonus, sidesteps unsigned-print semantics (a `u32` with the high bit set) entirely for
   this slice.
7. **IR carries width and signedness; the backend derives the register class and op
   signedness.** `IrType` grows so an integer value knows its `bits` and whether it is
   signed (e.g. `IrType::Int { bits, signed }`), because width and signedness are semantic
   and a future WASM lowering needs them too. The QBE register class (`w` for <=32, `l` for
   64) and the choice of signed vs unsigned QBE op are **derived** in the backend, not baked
   into the IR. `Ptr` stays opaque (invariant). `bool` stays a distinct type (`IrType::Bool`
   -> `w`); it is not folded into the integer scheme (you cannot do arithmetic on it).
8. **Sub-word representation is canonical.** A sub-word integer value lives in a `w` (or `l`)
   register with its out-of-width bits normalized (zero for unsigned, sign-extended for
   signed) after any operation that could leave them dirty (notably narrowing conversions and
   arithmetic that can overflow the width). This is the sharp codegen risk for the slice (the
   analogue of Slice 1's `bool`-width `RK1`): the implementer must define exactly where
   canonicalization happens so no two code paths disagree on a value's high bits. Keep it
   simple and correct over clever.

## Operator and conversion handling in the checker

The Slice 1 builtin table maps fixed typed signatures. Arithmetic, comparison, and the
conversion words are no longer expressible as single fixed-type entries; handle them the way
Slice 1 handles shuffles, as **structural, special-cased checker rules**, not a general
polymorphism system (honest polymorphic signatures + monomorphisation remain a Phase 4
formalisation):

- **Arithmetic `+ - * mod`:** pop two, require the same integer type, push that type.
- **Comparison `= < >`:** pop two, require the same integer type, push `bool`; record the
  operand signedness so the backend picks the right compare op.
- **Conversion `>iN`/`>uN`:** a word whose name is `>` immediately followed by a known
  integer type name; pop one integer type, push the named target type. Because Sooth tokens
  are whitespace-delimited, `>i32` is a single word distinct from `>` (comparison), so no
  lexer change is required; the checker recognizes the family by name.
- **Shuffles `dup drop swap over rot`:** unchanged, still structural and type-transparent
  (they already move whatever concrete types are present, so they move the new integer types
  for free).

Everything the checker cannot special-case stays a fixed-type builtin entry (`.` is
`( i64 -- )`).

## Type checking behaviour (new or changed vs Slice 1)

- Operand type checks now distinguish the eight integer types; a mixed-width arithmetic or
  comparison is a sharp, located error naming both types.
- Declared-output checking already exists (Slice 1); it now catches, e.g., a body that leaves
  `i64` where `u8` was declared, which is the common "you forgot to convert" error.
- `if` still requires `bool`; branch joins still unify on depth and per-slot type, now over
  the wider type set (an arm leaving `i32` and an arm leaving `u32` disagree).
- Unknown type names in effect comments are already reported; the known-type set simply grows
  to the eight integer types plus `bool`.
- The REPL carried stack tracks the new types per slot. All integer types plus `bool` remain
  8 bytes on the carried byte buffer for this slice (so the buffer marshalling size does not
  change), but each slot now records its true `Type`. Note the Slice-1 deferral comment in
  `lower_line` (carried slots currently load as a 64-bit `l`): revisit whether that must now
  thread the carried type so a sub-word carried value is loaded/stored at its own width, or
  whether keeping the buffer slot 8 bytes wide and canonicalizing on use is sufficient. The
  implementer should decide and justify; correctness of a carried sub-word value across REPL
  lines is an exit criterion.

## Frontend `Type` vs backend `IrType`

Keep them distinct so the IR stays backend-neutral. Frontend `Type` carries `{ bits, signed }`
for integers plus `Bool`. `IrType` carries the same width/signedness for its integer case plus
the existing `Bool`/`Ptr`. Lowering maps `Type -> IrType`. Do not collapse the integer types
into one width in the IR, and do not push the QBE register class (`w`/`l`) up into the IR.

## Diagnostics (tested as behaviour, per CLAUDE.md)

Each must produce the *right* error, asserted on message content and the type names, not just
a failure:

- **Mixed-width arithmetic (headline negative):** `i32 i64 +` reports the two differing integer
  types at the operator. This is the Slice 2 analogue of Slice 1's "`if` fed a non-bool".
- **Mixed-width or mixed-sign comparison:** `u8 i8 <` reports the differing operand types.
- **Declared-output requires conversion:** `: f ( -- u8 ) 5 ;` (literal is `i64`, declared
  `u8`) reports the mismatch against the declared effect.
- **Conversion of a non-integer:** `>i32` applied to a `bool` reports that the source is not an
  integer type.
- **Unknown type name:** an effect comment or conversion word naming an unknown type reports it
  (existing behaviour, extended to the new names; `>i128` should read as an unknown target).

(Exact wording is the implementer's; tests assert the salient substrings and the type names,
following the existing diagnostic style.)

## Goal and exit criteria

Deliver the fixed-width integer tower with explicit, target-only conversions, width- and
signedness-correct codegen, and homogeneous (no-implicit-promotion) arithmetic and comparison.

**Exit:**

1. All eight integer types (`i8 i16 i32 i64 u8 u16 u32 u64`) are usable in effect comments,
   arithmetic, comparison, and conversion; `bool` is unchanged; the Phase 0/1/Slice-1 goldens
   still pass (`gcd`/`factorial`/`lerp` still 5/120/30, the `sign` and `bool_abi` goldens still
   green).
2. `>iN`/`>uN` conversions work: widening sign/zero-extends by source signedness, narrowing
   truncates, and each is width-correct in the emitted QBE (a native binary produces the
   arithmetically expected value).
3. Homogeneous-operand enforcement is a sharp error: mixed-width arithmetic and mixed-width or
   mixed-sign comparison are reported diagnostics (the five above).
4. Signed vs unsigned comparison emits the correct QBE op (a signed and an unsigned comparison
   of the same bit pattern give the expected, differing boolean results in a run).
5. A sub-word carried value survives a REPL line boundary correctly (carried-stack type
   tracking over the wider type set).
6. **Dogfood: RGB-pack.** A small program that packs three `u8` channels into an `i32` via pure
   arithmetic (`r >i32 65536 >i32 * ...`, exercising `u8 -> i32` widening and homogeneous `i32`
   arithmetic) and unpacks one channel back to a `u8` via `/` and `mod` (exercising narrowing),
   printed via `>i64 .`. Compiled to a native binary producing a known value, and runnable in the
   REPL. Plus at least the headline negative golden (mixed-width arithmetic) proving a type error
   is a sharp diagnostic. Include a truncation golden (e.g. `511 >u8` wraps to `255`).

## Out of scope for Slice 2

Floating point (`f32`/`f64`, float literals, ordered comparison, int<->float conversion);
`i128`/`u128` (frontend double-word synthesis); **bitwise operators** (`and`/`or`/`xor`/`shl`/
`shr`/`sar` and the signed-vs-unsigned right-shift distinction, which gets its own slice);
the `*/` widening primitive; checked or saturating conversions; per-width or unsigned `.`
printing; structs/records; enums/ADTs and `match`; fixed-size arrays; optional and non-null
pointer types; the `Copy`/affine marker; polymorphic operator/shuffle *signatures* and
monomorphisation (Phase 4); any heap or move semantics (Phase 3).

## Current state / codebase anchors (post-Slice-1, on `main`)

- `src/ast.rs`: `Type` enum is currently `{ I64, Bool }` (`ast.rs:50`) with `Type::from_str`
  (`ast.rs:59`) and display (`ast.rs:67`); `StackEffect`/`TypedSlot` (`ast.rs:36`) carry slot
  types; `TermKind` includes `BoolLit` (`ast.rs:88`). The integer-carrying `Type::Int { bits,
  signed }` and the grown `from_str`/display land here (the lowest common ancestor of
  parser/checker/ir), unless a growth signal argues otherwise.
- `src/check.rs`: `Sig` (`check.rs:18`) and `builtin_table()` (`check.rs:35`) with fixed typed
  entries (`+`/`mod`/`<` at `check.rs:42`..`47`); `check_shuffle` (`check.rs:313`) is the model
  for the new structural operator/conversion handling; the `if`-wants-bool and join logic
  (`check.rs:286`); literal typing pushes `Type::I64`/`Type::Bool` (`check.rs:246`,`:250`).
- `src/ir.rs`: `IrType` currently `{ Int, Bool, Ptr }` (`ir.rs:31`) with `ir_type_of`
  (`ir.rs:41`); `lower_line` (`ir.rs:142`) with the Slice-1 carried-slot deferral comment
  (`ir.rs` prologue load around `ir.rs:158`); comparison/`if` lowering.
- `src/backend/qbe.rs`: emission of `l`/`w` typed values, comparison ops (signed today), `if`
  control flow. Must now derive `w`/`l` from `bits`, pick signed vs unsigned compare from
  `signed`, and emit conversion ext/truncate ops. This is where the sub-word canonicalization
  (decision 8) lives.
- `src/parser.rs`, `src/lexer.rs`: `>i32` is a single whitespace-delimited word, so no lexer
  change is expected; confirm the parser routes it to a term the checker can recognize by name.
- `examples/`: add `examples/rgb.sth` (the dogfood). Existing `gcd`/`factorial`/`lerp`/`sign`/
  `bool_abi` are unchanged.
- `tests/phase0.rs`, `tests/phase1.rs`: existing goldens unchanged (still green); new positive
  goldens (conversions, signed/unsigned comparison, the RGB-pack binary) and negative goldens
  (the five diagnostics) added; new REPL golden for a carried sub-word value.

## Test plan

- **Goldens:** the RGB-pack program compiled to a binary and run (known packed value, and an
  unpacked channel); a signed-vs-unsigned comparison program whose two results differ on the
  same bit pattern; a `>u8` truncation program (`511 >u8 >i64 .` prints `255`); the five negative
  diagnostics as negative goldens asserting the right text and type names; a REPL session leaving
  a sub-word value on the carried stack and using it on the next line; all Phase 0/1/Slice-1
  goldens still green.
- **Unit tests** beside each extended stage (per CLAUDE.md: happy path + at least one error/edge
  case), named `thing_condition_expected`:
  - checker: homogeneous arithmetic accepts same-type / rejects mixed-width; comparison signedness
    recorded; conversion accepts any integer source / rejects `bool`; declared-output requires
    conversion; unknown target type name; shuffle transparency over a new integer type.
  - ast/parser: `Type::from_str` for each new name; conversion-word recognition.
  - ir/backend: `Type -> IrType` for each width/signedness; a narrowing conversion emits the
    truncating op; a signed widening emits sign-extend and an unsigned widening zero-extend; a
    signed vs unsigned comparison emits the right QBE compare; sub-word canonicalization.
- Diagnostics are behaviour: every negative asserts the *right* message and the type names, not
  merely that it failed.
