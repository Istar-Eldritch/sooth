# Phase 2 Floats slice — condensed spec (delivered)

`f32`/`f64` with IEEE-754 arithmetic, ordered comparison, float literals, printing via the
type-directed `.`, and numeric-generalised int↔float conversions, width-correct in emitted QBE.
Mirrors the Slice-2 shape: table-driven, special-cased operator/conversion rules; **no**
polymorphism or promotion (that is Phase 4). Built on the `bool`+integer-tower compiler
(lexer → parser → checker → IR → QBE-emit → driver/REPL).

## Locked decisions (D1–D8)

- **D1.** Ship both `f32` and `f64` (two widths = table fill).
- **D2.** Homogeneous `+ - *` over "same numeric type"; `/` **float-only**; `mod`
  **integer-only** (float `mod` needs libm; `core` is `no_std`). No implicit promotion.
- **D3.** `< > =` are IEEE **ordered** compares → `bool`; `=` is exact bit-equality
  (`ceq`), never epsilon.
- **D4.** NaN/inf propagate silently (no trap/fault/static-reject). NaN detected via
  `x = x`; **no `isnan` primitive**.
- **D5.** Float literals `<digits>.<digits>` + optional exponent, default `f64`; digits
  required both sides of the dot so a literal can't collide with the `.` print word.
- **D6.** `.` is type-directed over every scalar and prints `f32`/`f64` via `%g`; there is
  no separate float-print word. *(Superseded, see banner: `.` is now type-directed over all scalars
  and `f.` was removed.)*
- **D7.** Conversions generalise target-only family to numeric: new `>f32`/`>f64`;
  `>iN`/`>uN` now accept a float source. Source must be numeric; `bool` source is an error.
- **D8.** IR carries float width (`IrType::Float { bits }`); backend derives register
  class (`s`/`d`). `Ptr` opaque, `bool` distinct, frontend `Type`/backend `IrType` separate.

## Requirements

**Frontend (R1–R5).** `Type::Float { bits: u8 }` (bits ∈ {32,64}, private) via a
`FLOAT_TYPES` table beside `INT_TYPES`; `from_name`/`name`/`Display` round-trip,
`f128`→None; add `is_float`/`is_numeric`. Lexer recognises `Token::Float(f64)` (digits
both sides, optional `[eE][+-]?digits`), tried before the `Word` fallthrough; `3.`/`.5`
fall through to `Word`; an out-of-range float literal saturates to infinity rather
than erroring (consistent with the slice's silent-inf semantics, D4, and with `f64`
parsing of this grammar never failing). Parser routes
`Token::Float` → `TermKind::FloatLit(f64)`. A `FloatLit` types as `Float { bits: 64 }`.

**Checker (R6–R12).** `check_operator` arithmetic arm: `+ - *` require equal + numeric;
`/` new, requires same float type; `mod` unchanged (same integer type). `= < >`
generalise to same-numeric → `bool`. Conversion recogniser adds `f32`/`f64` to
known-type set; source must be numeric. `.` prints floats (type-directed). `if` requires
`bool`; branch joins unify over float types. Shuffles stay type-transparent. Six sharp
located diagnostics naming the type(s) + rule: **X1** mixed int/float arith, **X2**
mixed float-width, **X3** `/` on ints, **X4** `mod` on floats, **X5** non-numeric
conversion source, **X6** unknown target (`>f128`).

**IR + backend (R13–R18).** `IrType::Float { bits }`, mapped in `ir_type_of`; IR never
spells `s`/`d`. Float literal lowers via `Instr::ConstF(Value, f64)` (Q1: separate
const path, backend never guesses int-vs-float). `width()` derives `s`/`d`. `Bin` runs
`add`/`sub`/`mul` at `s`/`d` width; `/` → `BinOp::Div` (float-only). `Cmp` emits ordered
float compares (`ceqs/d`, `clts/d`, `cgts/d`) → `Bool`/`w`. Sub-word canonicalization
never runs for floats. `Conv` extends to full numeric matrix: int→float
(`swtof`/`sltof`/`uwtof`/`ultof`, sub-word source widened first); float→float (`exts`
up, `truncd` down); float→int truncating toward zero (`stosi`/`dtosi`, then existing
narrow/canonicalize). Out-of-range/NaN→int unspecified (no checked/saturating).

**Print + REPL (R19–R21).** float printing via `.`, backed by a `"%g\n"` data string + `printf` passing a
`d`. `Instr::Load`/`Store` made width-aware (`loadd/stored`, `loads/stores` by
`IrType`) instead of hard-coded `loadl`/`storel`; `lower_line` prologue/epilogue
loads/stores float slots via the float op selected from the carried `Type` (no integer
`Conv`-relabel). `format_stack` renders a float slot via `f64::from_bits` (`f32` via
`f32::from_bits` on low 32 bits), not the `i64` bit pattern. `f32` in an 8-byte slot:
4-byte store/load, upper bytes stale (Q2).

## Non-functional

- **NF1.** Green (`fmt --check && clippy -D warnings && test`) at every phase.
- **NF2.** QBE only (no LLVM); `s`/`d` derived in backend, never in `IrType`; `Ptr`
  opaque, `bool` distinct, frontend/backend types separate; no JIT; `core` stays `no_std`.
- **NF3.** No regressions: all Phase 0/1/Slice-1/Slice-2 goldens pass; integer/`bool`
  behaviour byte-for-byte unchanged.
- **NF4.** `#[cfg(test)] mod tests` per extended stage (happy + error), named
  `thing_condition_expected`; each exit criterion a golden; negatives assert message
  substrings **and** type names.
- **NF5.** No premature abstraction (table + special-case, Slice-2 shape).

## Success criteria (S1–S8)

- **S1.** `f32`/`f64` usable in effects/arith/compare/conversion; prior goldens pass.
- **S2.** Literals parse, default `f64`; `3.`/`.5` rejected (lexer unit tests).
- **S3.** Float arith runs in a binary: `+ - * /`; `1.0 0.0 /`→inf, `0.0 0.0 /`→NaN, no
  trap; NaN detectable via `x = x`.
- **S4.** IEEE compare correct; NaN compare false.
- **S5.** Conversions width-correct: int→float, float→float both ways, float→int
  truncating (`3.9 >i64 .`→`3`, `-3.9 >i64 .`→`-3`).
- **S6.** Carried float survives a REPL line boundary and displays as its value.
- **S7.** Six diagnostics X1–X6 with message text + type names.
- **S8 (dogfood).** `examples/mean.sth` (mean of two ints as `f64`: `>f64`, float `/`,
  `.`; mean of 10 and 4 → `2.5`), native + REPL; plus the X1 headline negative.

## Scope

**In:** everything above. **Out (deferred):** epsilon compare; `isnan`/`isinf`; float
`mod`/`fmod`; FP trapping / NaN static rejection; rounding-mode control;
shortest-round-trip (Ryu/Grisu) printing; checked/saturating float→int; total order for
generic `sort`/`max`; `i128`/`u128`; bitwise ops; `*/`; structs/enums/`match`; arrays;
optional/non-null pointers; `Copy`/affine marker; polymorphic signatures (Phase 4); heap
or move semantics (Phase 3).

## Delivered phases

1. **Frontend type + float-literal lexing (R1–R4).** `Type::Float`+`FLOAT_TYPES`,
   `is_float`/`is_numeric`; `Token::Float`+`is_float_literal`; `TermKind::FloatLit`.
   (Typing/lowering deferred to later phases.)
2. **Checker operator/conversion rules (R5–R12).** Numeric arith/compare, float-only
   `/`, int-only `mod`, numeric conversion source/target, float printing via `.`, `FloatLit`
   typing, diagnostics X1–X6.
3. **Typed IR + float codegen (R13–R17).** `IrType::Float`, `Instr::ConstF`,
   `BinOp::Div`; `width()` `s`/`d`; float `Bin`/`Cmp` ordered compares; float const emit.
4. **Conversion-op lowering, full numeric matrix (R18).** int↔float / float↔float /
   float→int-truncate, reusing integer path as sub-step; per-cell IL tests + float→int
   truncation golden.
5. **Float printing via `.` + REPL carried float (R19–R21).** `%g\n` printf; width-aware
   load/store; `format_stack` `from_bits` display; carried-float REPL golden.
6. **Dogfood + goldens (S1–S8).** `examples/mean.sth`; positive goldens (both widths,
   inf/NaN, ordered+NaN compare, all conversion cells, truncation, mean binary); six
   negative diagnostics; REPL carried-float session; prior goldens stay green.

## Key risks (as landed)

- **NaN/inf are runtime values** the compile-error lever can't reach (D4);
  correctness rests on ordered float compares. Covered by running-binary goldens
  (S3/S4), not just IL mnemonic checks.
- **Conversion matrix** is the largest surface: integer path kept as-is and dispatched
  to as a sub-step; per-cell IL tests guard against integer-cell regressions.
- **Float-literal vs `.` print word:** digits-both-sides recogniser; tests that `3.14`/
  `1.5e-3` are `Float`, `3.`/`.5` are not, `5 .` stays `Int` + `Word(".")`.
- **REPL marshalling/display:** float slots need float store/load (else stale `i64`);
  `Load`/`Store` made width-aware; display via `from_bits`.

## Codebase touchpoints

`src/ast.rs` (Type, tables, `TermKind`), `src/lexer.rs` (`Token::Float`,
`is_float_literal`), `src/parser.rs` (`FloatLit` routing), `src/check.rs`
(operator/conversion/builtin/diagnostics), `src/ir.rs` (`IrType::Float`, `ConstF`,
`BinOp::Div`, `lower_line`/`lower_call`), `src/backend/qbe.rs` (`width`, `Bin`, `Cmp`,
`Conv`, `ConstF`, width-aware `Load`/`Store`, float print format), `src/repl.rs`
(`format_stack` display), `examples/mean.sth`, `tests/phase0.rs`, `tests/phase1.rs`.
