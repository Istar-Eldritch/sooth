# Phase 2 Slice 2 — the integer tower and explicit conversions (technical spec)

Derived from [phase2-slice2-brief.md](./phase2-slice2-brief.md). Read alongside
[../DESIGN.md](../DESIGN.md), [../ROADMAP.md](../ROADMAP.md), and [../CLAUDE.md](../CLAUDE.md).
This is a **craft** project: the plan below deliberately special-cases and hand-fills rather
than building a general polymorphism/inference framework (that is Phase 4). Everything the
brief locks is locked here; everything the brief defers stays deferred (see Scope).

## Problem statement

Slice 1 proved the typed-core spine: every stack slot carries a `Type`, and the checker
unifies type (not just depth) through word bodies and at branch joins, over exactly two types
(`i64`, `bool`). Two types are enough to make a mismatch *possible* but not enough to force the
machinery a real numeric type set demands. Slice 2 widens the type set along one axis, the
**fixed-width integer tower**, and pays for the three mechanisms a tower forces and Slice 1
could dodge:

1. **Width.** A sub-64-bit type (`i8`/`i16`/`i32` and the unsigned widths) means codegen must
   track a register class (`w` vs `l`) and keep sub-word values canonical.
2. **Signedness.** `u8`..`u64` mean widening is sign- vs zero-extend and comparison is
   `cslt`/`csgt` vs `cult`/`cugt`, chosen by operand type, not width alone.
3. **Explicit conversion.** With more than one integer type, moving between types is a
   deliberate, visible word, never an implicit promotion.

Floats, `i128`/`u128`, bitwise operators, and the `*/` widening primitive are separate axes and
are out of scope. The slice ends with a runnable dogfood (RGB-pack) plus goldens proving
width/signedness-correct codegen and sharp homogeneity diagnostics.

## The type set

Signed `i8 i16 i32 i64`, unsigned `u8 u16 u32 u64`, plus the existing `bool` (unchanged). The
eight integer types are **table-generated** from `{ bits ∈ 8/16/32/64 } × { signed ∈ true/false }`,
not eight hand-written enum variants, so breadth is cheap. `bool` is not in the numeric tower:
no arithmetic, no comparison-as-integer, no `>bool` conversion.

## Requirements

Each is independently verifiable. `Rn` requirements trace to the brief's locked decisions (D1–D8)
and the five diagnostics (X1–X5).

### Frontend type set

- **R1** (D1, type set). `Type` represents the eight integer types as `Type::Int { bits: u8,
  signed: bool }` (`bits ∈ {8,16,32,64}`) plus `Type::Bool`. The eight integer cases are
  produced from a table, not eight enum variants.
- **R2** (D1). `Type::from_name` resolves each of `i8 i16 i32 i64 u8 u16 u32 u64` and `bool`;
  every other name (including `i128`, `u128`, `f32`) returns `None`. `Display`/`name` renders
  each type back to exactly that spelling.
- **R3**. The parser resolves the eight new type names in effect-comment slots exactly as it
  resolves `i64`/`bool` today; an unknown type name in a slot stays the existing
  `error: unknown type ...` diagnostic (X5).

### Literals and conversions

- **R4** (D2, literals stay `i64`). An integer literal is always `Type::Int { bits: 64, signed:
  true }` (`i64`). There is no inference and no context-directed literal typing. A narrower or
  unsigned value is reached only by an explicit conversion word.
- **R5** (D5, conversion words). The eight words `>i8 >i16 >i32 >i64 >u8 >u16 >u32 >u64` each
  have effect `( <any integer type> -- <target> )`: pop one value of any integer type, push the
  named target type. They are recognised by name in the checker (a word whose name is `>`
  immediately followed by a known integer type name); no lexer or grammar change is required
  because `>i32` is one whitespace-delimited token distinct from `>`.
- **R6** (D5, conversion semantics). Conversion is width- and signedness-correct in emitted QBE:
  - Widening **sign-extends** when the **source** type is signed, **zero-extends** when the
    source is unsigned.
  - Narrowing **truncates**, keeping the low `bits` (well-defined wrap; silently truncating is
    the one deliberately-unsharp edge, documented per D5).
  - Same-width sign reinterpretation (`i32 >u32`, `u8 >i8`) is a bit-level no-op, a type relabel
    only.
- **R7** (D5, conversion source must be integer). A conversion applied to a `bool` is a type
  error (X4). There are no `>bool` conversions.

### Arithmetic and comparison

- **R8** (D3, homogeneous arithmetic). `+ - * mod` require both operands to be the **same**
  integer type and produce that type. Mixed integer types (e.g. `i32 i64 +`) is a sharp,
  located error naming both types (X1). There is no implicit promotion. A `bool` operand to
  arithmetic is a type error (existing behaviour, retained).
- **R9** (D4, homogeneous comparison). `= < >` require both operands to be the **same** integer
  type and produce `bool`. Mixed-width or mixed-sign operands are a sharp, located error naming
  both types (X2).
- **R10** (D4, signed vs unsigned compare). `<` and `>` emit a **signed** QBE compare
  (`cslt`/`csgt`) when operands are signed and an **unsigned** compare (`cult`/`cugt`) when
  operands are unsigned; `=` is signedness-agnostic (`ceq`). The compare op is chosen from the
  operand type carried through to the IR/backend.
- **R11** (shuffles unchanged). `dup drop swap over rot` stay structural and type-transparent;
  they move the new integer types with no new code (they already move whatever concrete `Type`
  is present).

### Print

- **R12** (D6, print stays `( i64 -- )`). `.` remains fixed-effect `( i64 -- )`. There is no
  per-width or unsigned printing; a narrower or unsigned value is printed by widening to `i64`
  first (`>i64 .`).

### IR and backend

- **R13** (D7, IR carries width and signedness). `IrType` grows an integer case carrying `bits`
  and `signed` (e.g. `IrType::Int { bits, signed }`) plus the existing `Bool` and `Ptr`.
  `Ptr` stays opaque (invariant). `bool` stays a distinct `IrType::Bool`, not folded into the
  integer scheme.
- **R14** (D7, register class derived in backend, not IR). The QBE base type (`w` for `bits ≤
  32`, `l` for `bits == 64`; `Bool`/`Ptr` unchanged) and the choice of signed vs unsigned QBE op
  are **derived in the backend** from `IrType`, never pushed up into the IR. The IR stays
  backend-neutral (a future WASM lowering reads `bits`/`signed`, not `w`/`l`).
- **R15** (D8, canonical sub-word representation). A sub-word integer value is kept canonical in
  its register (out-of-width bits zero for unsigned, sign-extended for signed) after any
  operation that can leave them dirty — narrowing conversions and width-overflowing arithmetic.
  Exactly one place per operation performs the canonicalization, so no two code paths disagree
  on a value's high bits. (This is the sharp codegen risk; see Risks.)

### REPL carried stack

- **R16** (carried type tracking). The REPL carried stack records each slot's true `Type` over
  the widened type set (the `types: Vec<Type>` already exists). A sub-word value left on the
  carried stack at a line boundary is loaded and used correctly on the next line: the
  implementer decides and documents whether the carried slot threads its own width or stays an
  8-byte slot canonicalized on use (see Open questions Q2), and the chosen path is proven by a
  golden.

## Diagnostics (tested as behaviour)

Each negative asserts the **right message** and the **type names**, not merely that it failed
(CLAUDE.md: diagnostics are behaviour). Exact wording is the implementer's, matching the existing
`error: ...` style in `check.rs`; tests assert salient substrings and the type names.

- **X1** (headline negative, mixed-width arithmetic). `: f ( -- i32 ) 1 >i32 5 +` (an `i32` and
  an `i64` to `+`) reports the two differing integer types at the operator. Slice 2's analogue
  of Slice 1's "`if` fed a non-bool".
- **X2** (mixed-width/sign comparison). `u8 i8 <` reports the differing operand types.
- **X3** (declared-output requires conversion). `: f ( -- u8 ) 5 ;` (literal is `i64`, declared
  `u8`) reports the mismatch against the declared effect (reuses Slice 1 declared-output
  checking over the wider type set).
- **X4** (conversion of a non-integer). `>i32` applied to a `bool` reports that the source is not
  an integer type.
- **X5** (unknown type name). An effect comment or conversion word naming an unknown type reports
  it (existing behaviour, extended): `>i128` reads as an unknown target.

## Non-functional requirements

- **NF1** (green). `cargo fmt --check && cargo clippy -- -D warnings && cargo test` passes at the
  end of every phase.
- **NF2** (invariants). Backend stays QBE (no LLVM). IR stays backend-neutral; `Ptr` stays
  opaque; `w`/`l` register class is never represented in `IrType`. No in-process JIT; the REPL
  keeps compiling to `.so` and `dlopen`-ing. `core` stays `no_std`.
- **NF3** (no regressions). All Phase 0/1/Slice-1 goldens stay green unchanged: `gcd`/`factorial`
  /`lerp` print `5`/`120`/`30`, `sign` prints `0`, `bool_abi` prints `1`, and the Phase 1 REPL
  sessions are unchanged.
- **NF4** (unit coverage per CLAUDE.md). Every extended stage function gets `#[cfg(test)] mod
  tests` beside it with a happy path plus at least one error/edge case, named
  `thing_condition_expected`.
- **NF5** (craft scope). No general polymorphism, inference, or trait system; conversions and
  operators are structural special cases alongside `check_shuffle`. Do not add abstraction beyond
  what the eight-type table and the special-case rules require.

## Success criteria (observable, mapping to the brief's six exit criteria)

- **S1 → Exit 1.** All eight integer types are usable in effect comments, arithmetic, comparison,
  and conversion; `bool` unchanged; NF3 goldens green.
- **S2 → Exit 2.** `>iN`/`>uN` conversions run correctly in a native binary: widening
  sign/zero-extends by source signedness, narrowing truncates, each width-correct.
- **S3 → Exit 3.** Mixed-width arithmetic and mixed-width/mixed-sign comparison are reported
  diagnostics (X1, X2, plus X3/X4/X5).
- **S4 → Exit 4.** A signed and an unsigned comparison of the same bit pattern give the expected,
  differing boolean results in a run (proves R10 codegen).
- **S5 → Exit 5.** A sub-word value survives a REPL line boundary correctly (R16).
- **S6 → Exit 6 (dogfood).** `examples/rgb.sth` packs three `u8` channels into an `i32` via pure
  homogeneous `i32` arithmetic (`r >i32 65536 >i32 * ...`, exercising `u8 -> i32` widening) and
  unpacks the **lowest** channel (blue) back to a `u8` via `rgb 256 >i32 mod >u8` (narrowing);
  printed via `>i64 .`; compiled to a native binary producing a known value and runnable in the
  REPL. Plus the X1 headline negative golden and a truncation golden (`511 >u8 >i64 .` prints
  `255`).

> Note on the dogfood arithmetic: the brief's unpack sketch mentions `/`, but `/` is not a Slice
> 1 builtin and integer division is not introduced by this slice (only `+ - * mod`). Resolution:
> unpack only the **lowest** channel (blue) via `rgb 256 >i32 mod >u8`, which needs no division.
> The middle and high channels cannot be isolated to a bare value without `/` (multiply only
> shifts up), so the dogfood does not extract them; blue alone still exercises the `i32 -> u8`
> narrowing the criterion wants. Do **not** add `/` to satisfy the dogfood.

## Scope and boundaries

### In scope

The eight fixed-width integer types; explicit target-only conversions `>i8`..`>u64`; homogeneous
(no-implicit-promotion) `+ - * mod` and `= < >`; signed/unsigned-correct comparison and
sign/zero-extend widening; well-defined truncating narrowing; width- and signedness-carrying IR;
backend-derived register class and op signedness with canonical sub-word representation; REPL
carried-type tracking over the wider set; the RGB-pack dogfood and the goldens above.

### Out of scope (mirrors the brief exactly — stays deferred)

Floating point (`f32`/`f64`, float literals, ordered comparison, int↔float conversion);
`i128`/`u128` (frontend double-word synthesis); **bitwise operators** (`and`/`or`/`xor`/`shl`/
`shr`/`sar` and the signed-vs-unsigned right-shift distinction, which gets its own slice); the
`*/` widening primitive; **integer division `/`**; **checked or saturating conversions**
(narrowing is silently truncating for now); per-width or unsigned `.` printing; structs/records;
enums/ADTs and `match`; fixed-size arrays; optional and non-null pointer types; the `Copy`/affine
marker; polymorphic operator/shuffle *signatures* and monomorphisation (Phase 4); any heap or move
semantics (Phase 3).

## Advisory solution approach

Not binding; the phased plan below is the contract. This is orientation.

- **`Type` (ast.rs).** Replace the flat `{ I64, Bool }` with `Int { bits, signed }` + `Bool`.
  Provide a private const table of `(name, bits, signed)` rows and drive `from_name`/`name` off
  it, so adding a width later is a table row. `Type::I64` sugar becomes `Type::Int { bits: 64,
  signed: true }`; keep a small constructor/helper if it reduces churn, but no new module.
- **Checker (check.rs).** Keep `+ - * mod` / `= < >` / `.` as the model split the brief names:
  `.` stays a fixed `builtin_table` entry (`( i64 -- )`); arithmetic, comparison, and the
  conversion family move to structural special cases handled the way `check_shuffle` is — a
  function that recognises the name, pops/pushes with the homogeneity rule, and emits a
  type-mismatch error naming both operand types. Conversion recognition: strip a leading `>` and
  ask `Type::from_name` for the rest; `Some(int type)` ⇒ conversion, source must be an integer.
- **IR (ir.rs).** Grow `IrType` to `Int { bits, signed }` + `Bool` + `Ptr`; `ir_type_of` maps
  `Type::Int { .. }` straight across. Arithmetic/comparison lowering carries the operand type into
  the instruction (or onto the result value's `IrType`) so the backend can pick signedness. Add a
  conversion instruction (e.g. `Instr::Conv(dst, src)` where `dst`/`src` carry their `IrType`) so
  the backend decides extend vs truncate vs no-op from the two widths and the source signedness.
- **Backend (qbe.rs).** `width()` derives `w`/`l` from `bits`. Comparison mnemonics switch on
  `signed`. Conversion lowers to QBE `extsb/extsh/extsw` (signed widen), `extub/extuh/extuw`
  (unsigned widen), a truncation via a mask/`copy` at the narrower `w` plus re-canonicalization,
  or a `copy`/no-op for same-width relabel. Define one canonicalization point (R15) and route
  every dirtying op through it.
- **REPL (repl.rs).** `types: Vec<Type>` already tracks slot types; decide Q2 and, if needed,
  thread the carried type into `lower_line`'s prologue load so a sub-word slot loads at its own
  width, or keep the 8-byte slot and canonicalize on use. `format_stack(&[i64])` interprets each
  slot as `i64`; confirm display of a canonicalized sub-word value is still correct (Q4).

## Codebase map (anchored, verified against `main`)

- **`src/ast.rs`** — `Type` enum `{ I64, Bool }` at `ast.rs:50`; `from_name` at `ast.rs:57`;
  `name` at `ast.rs:65`; `Display` at `ast.rs:73`; `TermKind` (`IntLit`/`BoolLit`/`Call`/`If`) at
  `ast.rs:86`. `TypedSlot`/`StackEffect` around `ast.rs:36`. This is the lowest common ancestor of
  parser/checker/ir; the `Type::Int { bits, signed }` growth and the table-driven
  `from_name`/`name` land here (R1–R2).
- **`src/check.rs`** — `Sig` at `check.rs:18`; `builtin_table()` at `check.rs:35` (fixed entries
  `+`/`-`/`*`/`mod`/`=`/`<`/`>`/`.` at `check.rs:42`–`49`); `check_term` literal typing pushes
  `Type::I64` at `check.rs:245` and `Type::Bool` at `check.rs:249`; the `if`-wants-`bool` and
  branch-join unify logic at `check.rs:279`; `check_shuffle` (the structural-rule model) at
  `check.rs:313`; `type_mismatch_error`/`branch_type_mismatch_error` helpers just above
  `check_term`. Arithmetic/comparison move out of `builtin_table` into structural rules; the
  conversion family is a new structural rule; `.` stays a table entry (R5, R8–R12).
- **`src/ir.rs`** — `IrType { Int, Bool, Ptr }` at `ir.rs:31`; `ir_type_of` at `ir.rs:41`;
  `lower_line` at `ir.rs:142` with the **Slice-1 carried-slot deferral comment at `ir.rs:160`**
  ("Every carried slot loads as `IrType::Int` (l-width) … must be revisited once a carried slot
  type can have a different width") — R16/Q2 live here; `BinOp`/`CmpOp` enums and the
  `FuncBuilder::lower_call` arithmetic/comparison arms (the `"+" | "-" | ...` and `"=" | "<" | ">"`
  blocks) where operand type must be carried; `lower_if`/phi at `ir.rs:328`. Grow `IrType`, add the
  conversion instruction, carry operand signedness (R13–R14).
- **`src/backend/qbe.rs`** — `width()` at `qbe.rs:44` (currently `Bool ⇒ w`, `Int`/`Ptr ⇒ l`);
  `emit_instr` at `qbe.rs:84`; comparison mnemonics `ceql`/`csltl`/`csgtl` at `qbe.rs:99`–`103`
  (signed today, fixed `l` operand width — the comment there notes it only ever compares `i64`);
  the `Store` `extuw` bool-widening path around `qbe.rs:140`. This is where `w`/`l` is derived from
  `bits`, signed vs unsigned compare is chosen, conversion ext/truncate ops are emitted, and
  sub-word canonicalization (R15) lives.
- **`src/parser.rs`** — `resolve_type` calls `Type::from_name`; grows for free once R2 lands. No
  grammar change: `>i32` is one whitespace-delimited `Token::Word`, routed to `TermKind::Call`
  like any other word, recognised by name in the checker.
- **`src/lexer.rs`** — no change expected (`>i32` already lexes as one `Word`; confirm no
  delimiter splits it — `>` is not in `is_delimiter`).
- **`src/repl.rs`** — `buf: Vec<i64>` at `repl.rs:163`; `types: Vec<Type>` at `repl.rs:167`;
  `format_stack(&[i64])` at `repl.rs:151`; `infer_line` call at `repl.rs:250`; `self.types =
  net_stack` at `repl.rs:302`. Carried-type tracking already exists; R16/Q2/Q4 are decided here.
- **`examples/`** — add `examples/rgb.sth` (dogfood). Existing `gcd`/`factorial`/`lerp`/`sign`/
  `bool_abi` unchanged.
- **`tests/phase0.rs`** — build-and-run goldens (`run_and_capture_stdout`) plus negative-diagnostic
  goldens (`check::check(...).expect_err`, asserting substrings). New positive goldens (conversions,
  signed/unsigned comparison, RGB-pack binary, truncation) and negatives (X1–X5) added here.
- **`tests/phase1.rs`** — scripted REPL sessions (`run_session`, assert on `stack:` lines). New
  golden: a session leaving a sub-word value on the carried stack and using it next line (S5).

## Open questions and risks

Lead risk first.

- **Q1 / RISK — sub-word canonicalization (D8, R15).** The sharp codegen risk of the slice, the
  analogue of Slice 1's `bool`-width handling. A sub-word value in a `w`/`l` register can carry
  dirty high bits after truncation or width-overflowing arithmetic; a signed compare, a widening
  extend, or a store then reads the wrong bits. **Mitigation:** define exactly one
  canonicalization point per dirtying operation (narrowing conversion; arithmetic that can
  overflow the width) so no two code paths disagree; add a backend unit test that a narrowing and
  a following widen/compare see canonical bits. Prefer simple-and-correct (e.g. always
  canonicalize after narrowing and after each sub-word arithmetic op) over clever minimal-mask
  analysis. Signed vs unsigned QBE op selection (`cslt` vs `cult`, `extsb` vs `extub`) is the
  paired hazard: getting signedness from the *source* type on widen and from the *operand* type on
  compare.
- **Q2 — REPL carried sub-word value (R16, ir.rs:160).** The Slice-1 deferral comment loads every
  carried slot as an `l`. Two viable paths: (a) keep the buffer slot 8 bytes wide and canonicalize
  the value on use each line (simplest, buffer marshalling unchanged); (b) thread the carried
  `Type` into `lower_line`'s prologue so a sub-word slot loads/stores at its own width. The
  implementer must pick one and justify it; correctness across a line boundary is exit criterion 5.
  Recommendation to evaluate: path (a) is likely sufficient because the checker already tracks the
  true `Type` and canonicalization (Q1) makes the stored 8-byte value's low `bits` authoritative —
  but the implementer verifies, does not assume.
- **Q3 (RESOLVED) — dogfood unpack without `/`.** `/` is not in the slice's operator set. Locked
  resolution: unpack only the **lowest** channel (blue) via `rgb 256 >i32 mod >u8`. The middle and
  high channels need division to isolate to a bare value, so the dogfood does not extract them;
  blue exercises the `i32 -> u8` narrowing the criterion needs. Do not add `/`.
- **Q4 — display of a carried unsigned/sub-word value.** `format_stack` interprets each slot as
  `i64`. A `u32` with the high bit set, or a canonicalized sub-word value, would display as its
  `i64` reinterpretation. The slice ships no unsigned printing (D6), so this is acceptable, but the
  implementer should confirm the REPL golden's expected output reflects the `i64` view and does not
  imply unsigned display.
- **Q5 — same-width relabel codegen.** `i32 >u32` / `u8 >i8` are bit no-ops (R6). Confirm they emit
  a `copy` (or nothing) and only change the value's `IrType`, and that a subsequent op reads the
  new signedness — a relabel from signed to unsigned that skips re-canonicalization would be a
  latent bug at the next widen/compare.

## Phased delivery plan

Sequenced by dependency so each phase is green (NF1) and the tower is provable incrementally.
Each phase names its files, its work, and its verification, so it can be executed without
re-exploring.

### Phase 1 — Frontend `Type` growth (standard, S)

**Files:** `src/ast.rs` (primary), `src/parser.rs`, `src/check.rs`, `src/ir.rs`, `src/repl.rs`
(mechanical `Type::I64`/`Type::Bool` match-arm updates only).

**Work.** Replace `Type::{ I64, Bool }` with `Type::Int { bits: u8, signed: bool }` + `Type::Bool`
(R1). Add a private table of `(name, bits, signed)` for the eight integer types; drive `from_name`
(R2) and `name`/`Display` off it. Keep an `i64` construction helper if it cuts churn. Update every
exhaustive `match` on `Type` across the crate to the new shape (checker literal typing at
`check.rs:245` still pushes `i64`; `ir_type_of` at `ir.rs:41` maps `Int { .. }` across;
`builtin_table` `.`/arith/cmp entries keep using `i64` for now). No behaviour change beyond the
type names now resolving.

**Verification / exit.** `cargo test` green (NF1); Slice-1 goldens unchanged (NF3). New ast unit
tests: `from_name` resolves each of the nine names / rejects `i128`,`f32` (X5 seed); `Display`
round-trips each name (`type_from_name_each_width_expected`, `type_unknown_name_none_expected`,
`type_display_roundtrip_expected`). Parser unit test: a slot typed `u8`/`i16` resolves
(`parse_slot_resolves_new_int_widths_expected`).

### Phase 2 — Checker operator and conversion rules (standard, M)

**Files:** `src/check.rs` (primary).

**Work.** Move `+ - * mod` and `= < >` out of `builtin_table` into structural special cases
handled like `check_shuffle` (`check.rs:313`): pop two, require the same integer type (reject
mixed / reject `bool`), push that type for arithmetic (R8) or `bool` for comparison (R9), and
record operand signedness so lowering can pick the compare op later (R10 — carried via the slot
`Type`, no new checker state needed). Add a conversion rule: a `Call(name)` where `name` starts
with `>` and the remainder resolves via `Type::from_name` to an integer type is a conversion — pop
one integer value (reject `bool`, X4; unknown remainder ⇒ unknown-type error, X5), push the target
(R5, R7). Keep `.` as the fixed `builtin_table` entry `( i64 -- )` (R12). Emit type-mismatch errors
naming **both** operand types (X1, X2), matching the existing `type_mismatch_error` style. Shuffles
stay untouched (R11).

**Verification / exit.** `cargo test` green; NF3 goldens green. New checker unit tests
(`thing_condition_expected`): homogeneous arithmetic accepts same type
(`check_arith_same_width_ok`) / rejects mixed width naming both (`check_arith_mixed_width_is_error`,
asserting `i32` and `i64` in the message); comparison homogeneous + rejects mixed sign
(`check_cmp_mixed_sign_is_error`); conversion accepts any integer source
(`check_conv_from_any_int_ok`) / rejects `bool` (`check_conv_of_bool_is_error`); declared-output
requires conversion (`check_declared_output_needs_conversion_is_error`, the `( -- u8 ) 5` case, X3);
unknown target `>i128` (`check_conv_unknown_target_is_error`, X5); shuffle transparency over a new
width (`check_shuffle_dup_u8_is_transparent`).

### Phase 3 — Typed IR + width/signedness-correct QBE codegen with canonicalization (HARD, L)

**Files:** `src/ir.rs` (primary), `src/backend/qbe.rs` (primary), `src/repl.rs`.

**Work.** Grow `IrType` to `Int { bits, signed }` + `Bool` + `Ptr` (R13); `ir_type_of` maps
`Type::Int` across. Carry operand type onto arithmetic and comparison results in
`FuncBuilder::lower_call` so the backend knows width and signedness (arithmetic result type = the
homogeneous operand type; comparison result stays `Bool` but the operands' signedness must reach
the backend — carry it via operand `IrType`, read at emit). In `qbe.rs`: `width()` derives `w` for
`bits ≤ 32`, `l` for `64` (R14, `Bool`/`Ptr` unchanged); arithmetic emits at the operand width
(`add`/`sub`/`mul`/`rem` on `w` vs `l`); comparison mnemonics switch signed (`cslt`/`csgt`) vs
unsigned (`cult`/`cugt`) on operand `signed`, `=` stays `ceq` at operand width (R10). Define the
single sub-word canonicalization point (R15): after each sub-word arithmetic op, re-normalize
out-of-width bits (sign-extend for signed, zero-extend/mask for unsigned). Resolve Q2 for the REPL
carried slot (default: keep 8-byte slots, canonicalize on use) and update the `ir.rs:160` deferral
comment to state the decision. **No conversion words yet** — this phase makes existing arithmetic/
comparison width-correct so a homogeneous non-`i64` program (fed by a later phase's conversions, or
by a hand-built IR unit test) lowers correctly.

**Verification / exit.** `cargo test` green; NF3 goldens green (i64 arithmetic still `l`, bools
still `w`). New ir unit tests: `ir_type_of` for each width/signedness
(`ir_type_of_each_width_expected`); arithmetic carries operand type
(`lower_add_u8_result_is_u8_typed`). New backend unit tests: `width()` maps each width
(`qbe_width_u8_is_w_expected`, `qbe_width_i64_is_l_expected`); a signed compare emits `cslt` and an
unsigned compare emits `cult` on the same shape (`emit_cmp_signed_uses_cslt`,
`emit_cmp_unsigned_uses_cult`); sub-word arithmetic canonicalizes
(`emit_subword_arith_canonicalizes`).

### Phase 4 — Conversion-op lowering (HARD, M)

**Files:** `src/ir.rs`, `src/backend/qbe.rs`.

**Work.** Add the IR conversion instruction (e.g. `Instr::Conv(dst, src)` with `dst`/`src` carrying
their `IrType`); lower each `>iN`/`>uN` call to it in `FuncBuilder::lower_call` (recognised by the
same `>`-prefix rule, pushing the target-typed result value). In the backend, emit from the two
widths and the **source** signedness (R6): widening ⇒ `extsb/extsh/extsw` (signed source) or
`extub/extuh/extuw` (unsigned source); narrowing ⇒ truncate to the target `w` then canonicalize
(R15); same-width ⇒ `copy`/relabel only (Q5). Route narrowing through the Phase 3 canonicalization
point so no path disagrees.

**Verification / exit.** `cargo test` green. New unit tests: a narrowing emits the truncating op
and canonicalizes (`emit_conv_narrow_truncates_and_canonicalizes`); a signed widen emits a
sign-extend (`emit_conv_signed_widen_sign_extends`); an unsigned widen emits a zero-extend
(`emit_conv_unsigned_widen_zero_extends`); a same-width relabel is a no-op
(`emit_conv_same_width_is_relabel`). At this point a full non-`i64` program (widen, arithmetic,
narrow, `>i64 .`) lowers end-to-end.

### Phase 5 — Dogfood, goldens, and REPL carried value (standard, M)

**Files:** `examples/rgb.sth` (new), `tests/phase0.rs`, `tests/phase1.rs`.

**Work.** Write `examples/rgb.sth` (S6): pack three `u8` channels into an `i32` via homogeneous
`i32` arithmetic (`>i32` widening, `+`/`-`/`*`), unpack the lowest channel (blue) via
`256 >i32 mod >u8` (Q3, no `/`), print via `>i64 .`, producing a known value. Add goldens to
`tests/phase0.rs`: RGB-pack binary runs and prints the known packed and unpacked values (S2, S6);
signed-vs-unsigned comparison program whose two results differ on the same bit pattern (S4);
truncation golden `511 >u8 >i64 .` prints `255` (S6); the five negatives X1–X5 as
`check::check(...).expect_err` goldens asserting the right text **and** the type names. Add a
`tests/phase1.rs` REPL session golden leaving a sub-word value on the carried stack and using it on
the next line (S5), with expected output reflecting the resolved Q2/Q4 behaviour.

**Verification / exit.** All six success criteria (S1–S6) observable. `cargo fmt --check && cargo
clippy -- -D warnings && cargo test` green (NF1). Every Phase 0/1/Slice-1 golden still green (NF3).
Slice 2 complete.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Frontend Type growth: replace Type::{I64,Bool} with table-generated Type::Int{bits,signed}+Bool; grow from_name/name/Display; mechanically update all Type matches across the crate. Slice-1 goldens unchanged.",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Checker operator and conversion rules: move + - * mod and = < > to structural special cases (homogeneous, no implicit promotion, record signedness); add the >iN/>uN conversion rule; keep . as a fixed ( i64 -- ) builtin; emit X1-X5 diagnostics naming both types.",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Typed IR + width/signedness-correct QBE codegen: grow IrType to Int{bits,signed}; carry operand type/signedness; derive w/l and signed vs unsigned compare ops in the backend; define the single sub-word canonicalization point; resolve the REPL carried-slot deferral. IR stays backend-neutral, Ptr opaque.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Conversion-op lowering: add the IR conversion instruction and lower >iN/>uN to it; emit sign/zero-extend on widen by source signedness, truncate-and-canonicalize on narrow, copy/relabel on same-width; route narrowing through the Phase 3 canonicalization point.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "Dogfood + goldens + REPL carried value: write examples/rgb.sth (pack u8 channels into i32, unpack via + - * mod and >u8, print via >i64 .); add positive goldens (RGB binary, signed-vs-unsigned compare, 511 >u8 truncation), the five negative diagnostic goldens, and a REPL sub-word carried-value session. Full green, no regressions.",
      "effort": "M",
      "difficulty": "standard"
    }
  ]
}
```
