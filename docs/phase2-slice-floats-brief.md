# Phase 2 Floats slice brief — `f32`/`f64`, IEEE arithmetic and conversions

Input for spec-writer. Phase 2 (typed core) is an epic; this is the floats slice, one of
the three numeric axes carved out of the integer tower (Slice 2). It builds directly on the
Phase 0/1/Slice-1/Slice-2 compiler already on `main` (lexer/parser/checker/IR/QBE-emit/
driver/REPL, with a type-carrying checker over `bool` and the fixed-width integer tower).
Read alongside [../DESIGN.md](../DESIGN.md), [../ROADMAP.md](../ROADMAP.md), and
[../CLAUDE.md](../CLAUDE.md). Everything here is scoped to this slice only.

Sequencing note: this jumps ahead of the aggregate slices (structs, enums/match, arrays).
Floats depend only on the Slice 1-2 spine, not on aggregates, so that is fine mechanically,
but Phase 2's exit (structs/enums/match) still needs the aggregate slices. This slice is a
numeric-axis detour, not progress toward closing the phase.

## Why this slice, and why it is worth doing alone

Slice 2 built the integer tower on three mechanisms: width, signedness, explicit conversion.
Floats look like "just more numeric types" but are a genuinely separate axis: the content
differs even though the structure rhymes. The new mechanisms a float type forces, none of
which the integer tower exercised:

1. **A new register class.** QBE `s` (float32) and `d` (float64) are distinct from `w`/`l`
   and are not derivable from the integer `{ bits, signed }` scheme. `width()` and the whole
   arithmetic/compare/conversion codegen path branch on int-vs-float.
2. **Float literals.** `3.14` needs a real lexer change and a new `FloatLit` term kind; the
   integer-literal path does not cover it. This is the first substantive lexer change since
   integer literals.
3. **IEEE-754 semantics, including NaN and infinity.** Float comparison is *ordered* with
   NaN comparing false to everything (including itself), and division makes NaN/inf
   producible for the first time. Slice 2's signed-vs-unsigned integer compares do not touch
   this.
4. **Float division `/`.** Deferred deliberately for integers, but well-defined for floats,
   so floats introduce `/`. Conversely there is no float `mod`.
5. **Int <-> float conversions.** These generalise the target-only conversion family from
   integer-only to numeric, but with entirely different QBE ops (`swtof`/`sltof`/`uwtof`/
   `ultof` for int->float, `stosi`/`dtosi` for float->int, `exts`/`truncd` for float->float)
   and their own rounding/truncation semantics. The existing conversion lowering assumes
   integer sources.

Notably, Slice 2's sharp risk (sub-word canonicalization) does **not** apply: floats fill
their register exactly.

## Type set for this slice

Add two float types to the existing set:

- `f32`, `f64`
- (unchanged) the integer tower `i8`..`i64` / `u8`..`u64`, and `bool`

Frontend `Type` gains a float case, e.g. `Type::Float { bits: u8 }` (`bits` one of 32/64),
alongside the existing `Type::Int { bits, signed }` and `Type::Bool`. Display renders
`f32`/`f64`. Keep the float types table-generated (a `FLOAT_TYPES` table, or fold into the
existing type table), mirroring the integer tower's `INT_TYPES` discipline, so `from_name`/
`name` stay table-driven. `IrType` gains a matching `Float { bits }` case.

## Decisions locked for this slice

1. **Both `f32` and `f64` in one slice.** Same as Slice 2 shipping the full integer breadth:
   the mechanism (float register class + literals + IEEE compare + `/` + conversions) is the
   work; two widths is table-fill on top.
2. **Homogeneous float arithmetic `+ - * /`, no `mod`.** `+ - *` generalise to "same numeric
   type" (int or float); `/` is **float-only** (integer division stays unsupported); `mod` is
   **integer-only** (float `mod` would need `fmod` from libm, and `core` is `no_std`, so it is
   out). No implicit int/float promotion: `3.0 5 +` (an `f64` and an `i64`) is a sharp type
   error, convert first.
3. **Comparison `< > =` are plain IEEE ordered compares, producing `bool`.** For floats the
   backend emits QBE's float compare mnemonics (inherently IEEE-ordered: any comparison
   against NaN is false). `=` is **exact** IEEE bit-equality (`ceq` on `s`/`d`), a documented
   footgun (`0.1 0.2 + 0.3 =` is false; `NaN = NaN` is false), **never epsilon**. Epsilon /
   approximate comparison is deferred to the standard library.
4. **NaN and infinity: silent IEEE propagation.** `1.0 0.0 /` = inf, `0.0 0.0 /` = NaN, no
   trap, no fault. This is the one place Sooth's compile-error lever does not reach: NaN/inf
   are inherently runtime, and short of dependent types (out of scope) nothing static
   prevents them. No trapping (fights IEEE and the numerical use case, and needs FP-control
   codegen against "the joy is the language"). NaN is user-detectable in-language via `x = x`
   (false only for NaN), so **no `isnan` primitive**; `isinf` and any total-ordering helper
   are deferred to the stdlib. Document the propagation loudly.
5. **Float literals `<digits>.<digits>` with optional exponent, defaulting to `f64`.**
   `3.14`, `1.5e-3`, `1.0e9`. Default type is `f64` (mirrors integer literals defaulting to
   the widest, `i64`); get an `f32` via `>f32`. **Digits are required on both sides of the
   dot** (`3.0` and `0.5` are legal; `3.` and `.5` are not) so a float literal cannot collide
   with the `.` print word. A new `TermKind::FloatLit(f64)` and a lexer change carry this.
6. **A distinct `f.` print word `( f64 -- )`; `.` stays `( i64 -- )`.** Do not overload `.`
   to accept `i64` or `f64` (that would make it polymorphic, which the language avoids
   everywhere). Print an `f32` via `>f64 f.`. `f.` needs a float print intrinsic in the
   runtime alongside the existing integer print; format is a readable `%g`-style rendering
   (shortest-round-trip / Ryu is deferred).
7. **Conversions generalise the target-only family to numeric.** New `>f32`/`>f64`; the
   existing `>iN`/`>uN` now also accept a float source. Semantics:
   - **int -> float:** exact when representable, else round to nearest (`swtof`/`sltof`/
     `uwtof`/`ultof` by source width/signedness).
   - **float -> float:** `f32 >f64` is exact (`exts`), `f64 >f32` rounds to nearest
     (`truncd`).
   - **float -> int:** **truncates toward zero** (`stosi`/`dtosi`), matching C. Out-of-range
     or NaN -> int is **unspecified** this slice (no checked/saturating conversions,
     consistent with the integer tower's silently-truncating narrowing).
   - The conversion recogniser (a word whose name is `>` + a known type name) simply grows
     the known-type set to include `f32`/`f64`. The source must be **numeric** (int or float);
     a `bool` source stays an error (Slice 2's "source must be integer" generalises to
     "source must be numeric").
8. **IR carries the float width; the backend derives the register class.** `IrType::Float
   { bits }` maps to QBE `s`/`d` in `width()`, kept out of the IR itself (backend-neutral, as
   with the integer register class). `Ptr` stays opaque; `bool` stays distinct. Frontend
   `Type` and backend `IrType` stay distinct.

## Operator and conversion handling in the checker

Extend the Slice 2 structural operator/conversion rules (modelled on `check_shuffle`, not a
general polymorphism system) to numerics:

- **Arithmetic `+ - *`:** pop two, require the **same numeric type** (int or float), push it.
- **`/`:** pop two, require the **same float type**, push it. An integer operand is a sharp
  error ("`/` requires float operands" / integer division is not supported).
- **`mod`:** pop two, require the **same integer type**, push it (unchanged). A float operand
  is a sharp error.
- **Comparison `= < >`:** pop two, require the **same numeric type**, push `bool`. Record
  whether the operands are int (and their signedness, as Slice 2 does) or float, so the
  backend picks the signed / unsigned / float compare op.
- **Conversion `>iN`/`>uN`/`>fN`:** pop one **numeric** value (int or float), push the named
  target; reject a `bool` source. Recognised by name, target-set grown with `f32`/`f64`.
- **Shuffles `dup drop swap over rot`:** unchanged, structural and type-transparent; they
  move floats for free.

`f.` is a new fixed-type builtin `( f64 -- )`; `.` stays `( i64 -- )`.

## Type checking behaviour (new or changed vs Slice 2)

- Operand type checks span int and float; a mixed int/float or mixed-float-width
  arithmetic/comparison is a sharp, located error naming both types.
- `if` still requires `bool`; branch joins still unify on depth and per-slot type, now over
  the float types too (an arm leaving `f32` and an arm leaving `f64` disagree).
- Unknown type names already reported; the known-type set grows with `f32`/`f64`.
- The REPL carried stack now tracks float slots. Two wrinkles for the implementer to resolve
  and justify (correctness across a line boundary is an exit criterion):
  - **Marshalling:** the carried byte buffer is 8-byte slots. An `f64` is 8 bytes; storing/
    loading a float slot must use a float store/load (`stored`/`loadd`, `stores`/`loads`), not
    the integer `storel`/`loadl`, or the bits round-trip but the relabel-to-true-type must
    pick the float load. Decide whether to thread the carried `Type` (as the Slice-2
    carried-slot work began) so a float slot loads as a float.
  - **Display:** the REPL stack display currently interprets each slot as `i64`. A carried
    float shown as its `i64` bit pattern is meaningless (worse than Slice 2's unsigned-display
    caveat). Prefer rendering a float slot as its float value in the stack display (the
    per-slot `Type` is available), or defer float display with a documented caveat; the
    implementer picks and justifies.

## Diagnostics (tested as behaviour, per CLAUDE.md)

Each must produce the *right* error, asserted on message content and the type names, not just
a failure:

- **Mixed int/float arithmetic (headline negative):** `3.0 5 +` (an `f64` and an `i64`)
  reports the two differing types at the operator. The floats analogue of Slice 2's
  mixed-width headline.
- **Mixed float-width arithmetic/comparison:** an `f32` and an `f64` to `+` or `<` reports the
  differing types.
- **`/` on integers:** `5 3 /` reports that `/` requires floats / integer division is
  unsupported.
- **`mod` on floats:** `3.0 2.0 mod` reports that `mod` requires integers.
- **Conversion of a non-numeric:** `>f64` applied to a `bool` reports that the source is not a
  numeric type.
- **Unknown type name:** unchanged behaviour, extended to the float names (`>f128` reads as an
  unknown target).

(Exact wording is the implementer's; tests assert the salient substrings and the type names,
following the existing diagnostic style.)

## Goal and exit criteria

Deliver `f32`/`f64` with IEEE arithmetic (`+ - * /`), IEEE ordered comparison, float literals,
`f.` printing, and numeric-generalised conversions, all width-correct in the emitted QBE.

**Exit:**

1. `f32` and `f64` are usable in effect comments, arithmetic, comparison, and conversion;
   integer/`bool` behaviour is unchanged; all Phase 0/1/Slice-1/Slice-2 goldens still pass.
2. Float literals parse (`3.14`, `1.5e-3`), default to `f64`, and `3.` / `.5` are rejected so
   they cannot collide with the `.` print word.
3. Float arithmetic runs correctly in a native binary: `+ - *` on both widths, and `/`
   (including that `1.0 0.0 /` yields inf and `0.0 0.0 /` yields NaN without trapping, and
   NaN is detectable via `x = x`).
4. IEEE comparison is correct in a run: an ordered comparison gives the expected boolean, and
   a comparison involving NaN is false (e.g. `x = x` is false for a NaN produced by `0.0 0.0
   /`).
5. Conversions run width-correct in a native binary: int->float, float->float (both
   directions), and float->int truncating toward zero (`3.9 >i64 .` prints `3`, `-3.9 >i64 .`
   prints `-3`).
6. A carried float survives a REPL line boundary correctly (carried-stack float marshalling).
7. Homogeneous-operand enforcement is a sharp error: the six diagnostics above.
8. **Dogfood: float mean.** A small program that computes the mean of integer inputs as an
   `f64` (converting via `>f64`, dividing with float `/`) and prints it with `f.`, exercising
   int->float conversion, float division, and float printing in one honest program. Compiled
   to a native binary producing a known value (e.g. mean of `10` and `4` prints `2.5`), and
   runnable in the REPL. Plus the headline negative golden (mixed int/float arithmetic).

## Out of scope for this slice

Epsilon / approximate comparison (deferred to the stdlib); an `isnan`/`isinf` primitive (NaN
is user-detectable via `x = x`; `isinf` waits for the stdlib); float `mod`/`fmod`; FP trapping
or NaN/inf static rejection; rounding-mode control; shortest-round-trip (Ryu/Grisu) printing;
checked or saturating float->int conversion; a **total order** over floats for a generic
`sort`/`max` (there is no generic `Ord` bound yet; revisit when generics land in Phase 4,
Rust-`total_cmp` style). Also still out: `i128`/`u128`; bitwise operators; the `*/` widening
primitive; structs/records; enums/ADTs and `match`; fixed-size arrays; optional and non-null
pointer types; the `Copy`/affine marker; polymorphic operator/shuffle *signatures* (Phase 4);
any heap or move semantics (Phase 3).

## Current state / codebase anchors (post-Slice-2, on `main`)

- `src/ast.rs`: `Type` is `{ Int { bits, signed }, Bool }` driven by the `INT_TYPES` table,
  with `from_name`/`name`/`Display`; `StackEffect`/`TypedSlot`; `TermKind` includes `IntLit`/
  `BoolLit`/`Call`/`If`. The float case `Type::Float { bits }`, a `FLOAT_TYPES` table (or a
  fold), the grown `from_name`/`name`, and a new `TermKind::FloatLit(f64)` land here (the
  lowest common ancestor of parser/checker/ir).
- `src/lexer.rs`: integer-literal lexing. Add float-literal lexing (`<digits>.<digits>` +
  optional exponent, digits required both sides). This is the main lexer change.
- `src/parser.rs`: routes literals/operators/conversions to terms. Route `FloatLit`; confirm
  `>f32`/`>f64` and `f.` are recognised (whitespace-delimited words, no grammar change beyond
  the literal).
- `src/check.rs`: `check_operator` (the Slice 2 structural operator/conversion rule modelled
  on `check_shuffle`) is where `/` (float-only), `mod` (int-only), the numeric-generalised
  `+ - *` / comparison, and the numeric-source conversion recogniser extend; the fixed
  builtin table gains `f.` `( f64 -- )`; literal typing pushes `Type::Float { bits: 64 }` for
  a `FloatLit`.
- `src/ir.rs`: `IrType` is `{ Int { bits, signed }, Bool, Ptr }` with `ir_type_of`; add
  `Float { bits }`. `lower_line` (carried-slot marshalling, with the Slice-1/2 relabel logic)
  must handle float slots. Conversion lowering (`Instr::Conv`) extends to int<->float and
  float<->float.
- `src/backend/qbe.rs`: `width()` derives `w`/`l` today; add `s`/`d` from `Float { bits }`.
  Arithmetic emits float add/sub/mul/div (and `/` only exists for floats); comparison emits
  the float compare mnemonics for float operands; `emit_conv` gains the int<->float and
  float<->float ops (`swtof`/`sltof`/`uwtof`/`ultof`/`stosi`/`dtosi`/`exts`/`truncd`);
  sub-word canonicalization is untouched (floats fill their register). A float print intrinsic
  backs `f.`.
- `src/driver.rs` / runtime print path: the integer `.` print intrinsic gains a float sibling
  for `f.` (readable `%g`-style).
- `examples/`: add `examples/mean.sth` (the dogfood). Existing examples unchanged.
- `tests/phase0.rs`, `tests/phase1.rs`: existing goldens unchanged (still green); new positive
  goldens (float arithmetic incl. inf/NaN, comparison, conversions both directions, the mean
  binary) and negative goldens (the six diagnostics); a REPL golden for a carried float.

## Test plan

- **Goldens:** the mean program compiled to a binary and run (known value, e.g. `2.5`); float
  arithmetic on both widths; `/` producing inf (`1.0 0.0 /`) and NaN (`0.0 0.0 /`) with NaN
  detected via `x = x`; an ordered comparison and a NaN comparison; int->float, float->float
  both directions, and float->int truncation (`3.9 >i64 .` -> `3`, `-3.9 >i64 .` -> `-3`); the
  six negative diagnostics asserting the right text and type names; a REPL session leaving a
  float on the carried stack and using it on the next line; all prior goldens still green.
- **Unit tests** beside each extended stage (per CLAUDE.md: happy path + at least one error/
  edge case), named `thing_condition_expected`:
  - lexer: `3.14` / `1.5e-3` lex as a float literal; `3.` and `.5` are rejected (or lex as int
    - `.`); an integer still lexes as an integer.
  - ast/parser: `Type::from_name` for `f32`/`f64`; `FloatLit` parsing.
  - checker: same-numeric-type arithmetic accepts / mixed int-float rejects; `/` rejects ints;
    `mod` rejects floats; comparison records float-ness; conversion accepts a float source and
    rejects a `bool`; unknown float target name; branch-join over float types.
  - ir/backend: `Type -> IrType` for `f32`/`f64`; float arithmetic and `/` emit the float ops;
    a float compare emits the float compare mnemonic; each conversion cell (int<->float,
    float<->float, float->int truncate) emits the right QBE op; `f.` emits the float print.
- Diagnostics are behaviour: every negative asserts the *right* message and the type names,
  not merely that it failed.
