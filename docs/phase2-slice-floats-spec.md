# Phase 2 Floats slice — technical specification

`f32`/`f64` with IEEE-754 arithmetic, ordered comparison, float literals, `f.`
printing, and numeric-generalised int↔float conversions, all width-correct in the
emitted QBE.

This is the spec for the floats slice of Phase 2 (typed core). It implements
[phase2-slice-floats-brief.md](./phase2-slice-floats-brief.md) and builds directly
on the Phase 0/1/Slice-1/Slice-2 compiler on `main` (lexer → parser → checker → IR →
QBE-emit → driver/REPL, a type-carrying checker over `bool` and the fixed-width
integer tower). Read alongside [../DESIGN.md](../DESIGN.md),
[../ROADMAP.md](../ROADMAP.md), and [../CLAUDE.md](../CLAUDE.md).

**Craft discipline.** This mirrors the Slice-2 shape: special-cased, table-driven
rules modelled on `check_operator`/`check_shuffle`, *not* a general polymorphism or
promotion system (that is Phase 4). Do not invent scope beyond the brief. Every
decision the brief locks (D1–D8) is locked; everything it defers stays deferred (see
[Scope](#scope-and-boundaries)).

---

## Problem statement

Slice 2 built the integer tower on three mechanisms: width, signedness, explicit
conversion. Floats look like "more numeric types" but are a genuinely separate axis;
the structure rhymes, the content differs. Four things a float type forces that the
integer tower never exercised:

1. **A new register class.** QBE `s` (float32) and `d` (float64) are distinct from
   `w`/`l` and are *not* derivable from the integer `{ bits, signed }` scheme.
   `width()` and the arithmetic/compare/conversion codegen branch on int-vs-float.
2. **Float literals.** `3.14` needs a real lexer change and a new `FloatLit` term
   kind; the integer-literal path does not cover it.
3. **IEEE-754 semantics, including NaN and infinity.** Float comparison is *ordered*
   with NaN comparing false to everything (including itself), and division makes
   NaN/inf producible for the first time. This is the one place Sooth's compile-error
   lever cannot reach: NaN/inf are inherently runtime values.
4. **Int↔float conversions.** These generalise the target-only conversion family
   from integer-only to numeric, with entirely different QBE ops and their own
   rounding/truncation semantics; the existing conversion lowering assumes integer
   sources.

Notably, Slice 2's sharp risk (sub-word canonicalization) does **not** apply: floats
fill their register exactly (`s` is 4 bytes, `d` is 8), so there are no dirty high
bits to normalise.

The deliverable: `f32`/`f64` usable in effect comments, literals, homogeneous
`+ - * /` (float `/` is in, no `mod`), IEEE ordered `< > =`, `f.` printing, and
numeric-generalised conversions, all width-correct in the emitted QBE, with the six
homogeneity/source diagnostics as sharp located errors, and integer/`bool` behaviour
unchanged.

---

## Locked decisions (from the brief)

Restated so requirements can trace to them. Do not reopen these.

- **D1.** Both `f32` and `f64` ship in one slice (two widths is table-fill on the one
  mechanism).
- **D2.** Homogeneous arithmetic `+ - *` over "same numeric type" (int or float); `/`
  is **float-only**; `mod` stays **integer-only** (float `mod`/`fmod` needs libm and
  `core` is `no_std`). No implicit int/float promotion.
- **D3.** Comparison `< > =` are plain IEEE ordered compares producing `bool`; `=` is
  **exact** IEEE bit-equality (`ceq`), a documented footgun, **never** epsilon.
- **D4.** NaN and infinity propagate silently (no trap, no fault, no static
  rejection); NaN is user-detectable via `x = x` (false only for NaN), so **no
  `isnan` primitive**.
- **D5.** Float literals are `<digits>.<digits>` with an optional exponent, defaulting
  to `f64`; **digits required on both sides of the dot** so a literal cannot collide
  with the `.` print word.
- **D6.** A distinct `f.` print word `( f64 -- )`; `.` stays `( i64 -- )`. `.` is not
  overloaded (the language avoids polymorphic words). Print an `f32` via `>f64 f.`.
- **D7.** Conversions generalise the target-only family to numeric: new `>f32`/`>f64`,
  and the existing `>iN`/`>uN` now also accept a float source. Source must be
  **numeric** (int or float); a `bool` source stays an error.
- **D8.** IR carries the float width (`IrType::Float { bits }`); the backend derives
  the register class (`s`/`d`), exactly as it derives `w`/`l` for ints. `Ptr` stays
  opaque; `bool` stays distinct; frontend `Type` and backend `IrType` stay separate.

---

## Requirements

Numbered, independently verifiable, traceable to D1–D8 and the six diagnostics
(X1–X6). Each is testable by a unit test beside the stage and/or a golden.

### Frontend: types and literals

- **R1 (D1, D8).** `ast::Type` gains a float case `Type::Float { bits: u8 }` with
  `bits ∈ {32, 64}`, alongside the existing `Type::Int(IntType)` and `Type::Bool`.
  The two float types are **table-generated** (a `FLOAT_TYPES` table `[("f32", 32),
  ("f64", 64)]`, or folded into a shared type table) mirroring `INT_TYPES`, so
  `from_name`/`name` stay table-driven and adding a width is one row. `bits` is
  private (constructable only via `from_name`), matching `IntType`.
- **R2 (D1).** `Type::from_name` resolves `"f32"`/`"f64"` to the float cases and
  continues to reject everything off-table (`"f128"` → `None`). `Type::name`/`Display`
  round-trips `f32`/`f64`. A `Type::is_float()` predicate is added; a `Type::is_numeric()`
  predicate (int-or-float) is added for the operator/conversion rules. Parser effect-comment
  slot resolution picks up `f32`/`f64` for free through `from_name`; an unknown float-ish
  name keeps the existing `error: unknown type` diagnostic.
- **R3 (D5).** The lexer recognises a float literal `<digits>.<digits>` with an
  optional `e`/`E` exponent (`3.14`, `0.5`, `1.5e-3`, `1.0e9`) as a new
  `Token::Float(f64)`, tried before the `Word` fallthrough. **Digits are required on
  both sides of the dot**: `3.` and `.5` are **not** float literals (they fall through
  to `Word` and error later as unknown words), so no literal can collide with the `.`
  print word. An integer with no dot still lexes as `Token::Int`. A magnitude beyond
  `f64` range parses to `inf`/`0.0` (Rust's `f64::from_str` never errors on this
  grammar) rather than a lex error; this matches the language's own
  silent-inf-propagation semantics (D4) instead of fighting them.
- **R4 (D5).** The parser routes `Token::Float(v)` to a new `TermKind::FloatLit(f64)`
  (analogous to `IntLit`). No other grammar change: `>f32`/`>f64`/`f.` are ordinary
  whitespace-delimited words already tokenised as `Word`.

### Checker: operators, conversions, print, literal typing

- **R5 (D5).** A `FloatLit` types as `Type::Float { bits: 64 }` (default `f64`); no
  inference, exactly as an `IntLit` types as `i64`.
- **R6 (D2).** `check_operator`'s arithmetic arm `+ - *` generalises from "same
  integer type" to "same **numeric** type" (int or float): pop two, require equal and
  numeric, push that type. A mixed int/float or mixed-float-width pair is a sharp
  located error naming both types (**X1**, **X2**).
- **R7 (D2).** `/` is a new operator: pop two, require the **same float type**, push
  it. An integer (or mixed) operand is a sharp located error stating `/` requires
  floats / integer division is unsupported (**X3**).
- **R8 (D2).** `mod` stays **integer-only**: pop two, require the same integer type,
  push it. A float (or mixed) operand is a sharp located error stating `mod` requires
  integers (**X4**).
- **R9 (D3).** `= < >` generalise to "same numeric type", producing `bool`. The
  checker records enough to let the backend pick the compare op: for an integer pair,
  signedness (as Slice 2 does); for a float pair, float-ness. (This is already carried
  by the operand `IrType` in IR; no extra checker state beyond the type equality
  check.) A mixed pair is a sharp located error naming both types (**X2**).
- **R10 (D7).** The conversion recogniser (`>` + a known type name) grows its
  known-type set to include `f32`/`f64`, so `>f32`/`>f64` are recognised and `>iN`/`>uN`
  are unchanged in spelling. The source must be **numeric** (int or float); a `bool`
  source is a sharp error stating the source must be numeric (**X5**). An unknown
  target name (`>f128`) keeps the existing unknown-type diagnostic (**X6**).
- **R11 (D6).** `f.` is a new fixed-type builtin `( f64 -- )` in `builtin_table`,
  beside the unchanged `.` `( i64 -- )`. Neither is overloaded.
- **R12 (D3).** `if` still requires `bool`; branch joins unify on depth and per-slot
  type over the float types too (an arm leaving `f32` and an arm leaving `f64`
  disagree, reported as a branch-join type mismatch). Shuffles (`dup drop swap over
  rot`) stay structural and type-transparent and move floats for free (no change).

### IR and backend: float codegen

- **R13 (D8).** `ir::IrType` gains `Float { bits: u8 }`; `ir_type_of` maps
  `Type::Float { bits }` → `IrType::Float { bits }`. `Bool` stays distinct, `Ptr`
  stays opaque. The IR never spells `s`/`d`.
- **R14 (D5, D8).** A float literal lowers to a float constant carrying its `f64`
  value and float `IrType` (the existing `Instr::Const(Value, i64)` carries an
  integer bit-payload; add a float-const path, e.g. `Instr::ConstF(Value, f64)`, so
  the backend emits a QBE float constant rather than reinterpreting an `i64`). The
  literal's `IrType` is `Float { bits: 64 }`.
- **R15 (D8).** The backend `width()` derives `s` from `Float { bits: 32 }` and `d`
  from `Float { bits: 64 }`, alongside the existing `w`/`l` for ints and `Ptr`. This
  is the only place the register class is spelled.
- **R16 (D2).** `Instr::Bin` emits the float arithmetic mnemonics (`add`/`sub`/`mul`
  are shared spellings but run at `s`/`d` width; `div` is emitted for `/`) when the
  result type is `Float`. `/` lowers to a new `BinOp::Div` present **only** for float
  operands (there is no integer `/`; the checker guarantees it). Sub-word
  canonicalization is untouched and never runs for floats (floats fill their
  register).
- **R17 (D3).** `Instr::Cmp` emits QBE's float compare mnemonics for float operands
  (`ceqs`/`ceqd` for `=`, `clts`/`cltd` for `<`, `cgts`/`cgtd` for `>`, i.e. the
  ordered forms that are false against NaN), selected by the operand `IrType` being
  `Float`; the integer signed/unsigned selection is unchanged. The result stays
  `Bool`/`w`.
- **R18 (D7).** `Instr::Conv` lowering extends from integer-only to the full numeric
  matrix, selected by the source/target `IrType`s (the frontend never spells the QBE
  op):
  - **int → float:** `swtof`/`sltof` (signed 32/64 source) and `uwtof`/`ultof`
    (unsigned 32/64 source); a sub-word integer source is first widened to its 32/64
    carrier by source signedness (reusing the existing widen path) then converted.
    Exact when representable, else round to nearest (QBE/hardware default).
  - **float → float:** `f32 >f64` is exact (`exts`); `f64 >f32` rounds to nearest
    (`truncd`).
  - **float → int:** truncates toward zero (`stosi`/`dtosi` by source width to the
    32/64 integer carrier, then the existing narrow/canonicalize path for a sub-word
    target). Out-of-range or NaN → int is **unspecified** this slice (no
    checked/saturating conversion), consistent with the integer tower's silently
    truncating narrowing.

### Runtime print and REPL

- **R19 (D6).** A float print intrinsic backs `f.`: emit a `%g`-style readable
  rendering (a `data` format string `"%g\n"` and a `printf` call passing the value as
  a `d`). Shortest-round-trip (Ryu/Grisu) is deferred. `.`'s integer print path is
  unchanged. An `f32` reaches `f.` only via `>f64` (checker-enforced), so the print
  intrinsic only ever handles `d`.
- **R20 (D8).** The REPL carried stack marshals a float slot correctly across a line
  boundary. The 8-byte buffer slot is retained; a float slot's store/load use the
  **float** store/load (`stored`/`loadd` for `f64`, `stores`/`loads` for `f32`), not
  the integer `storel`/`loadl`, so the bits round-trip *and* the value re-enters the
  next line as its true float `IrType` (not a stale `i64`). The per-slot `Type` the
  session already threads (`Session.types`) selects the float load in `lower_line`'s
  prologue and the float store in its epilogue; a float slot needs no integer
  `Conv`-relabel (that path is integer-only).
- **R21 (D8).** The REPL stack **display** renders a carried float slot as its float
  value, not its `i64` bit pattern (which would be meaningless, worse than Slice 2's
  unsigned-display caveat). `format_stack` (or its caller) reads the per-slot `Type`
  and reinterprets an `f64` slot via `f64::from_bits` (an `f32` slot via
  `f32::from_bits` on the low 32 bits) for display; integer/`bool` slots print
  unchanged.

---

## Non-functional requirements

- **NF1 — Green at every phase.** `cargo fmt --check && cargo clippy -- -D warnings &&
  cargo test` passes at the end of each delivery phase. Each phase is independently
  green and leaves floats provable so far as it goes.
- **NF2 — Invariants held.** Backend stays **QBE** (no LLVM). IR stays
  **backend-neutral**: the `s`/`d` register class is derived in the backend and never
  pushed into `IrType` (a WASM lowering reads `Float { bits }`, never `s`/`d`), exactly
  as `w`/`l` are kept out today. `Ptr` stays opaque; `bool` stays distinct; frontend
  `Type` and backend `IrType` stay separate. No in-process JIT; `core` stays `no_std`
  (this is *why* float `mod`/`fmod` is out — it would call libm).
- **NF3 — No regressions.** All Phase 0/1/Slice-1/Slice-2 goldens still pass unchanged
  (`tests/phase0.rs`, `tests/phase1.rs`, in-crate unit tests). Integer and `bool`
  behaviour is byte-for-byte unchanged.
- **NF4 — Test coverage per convention.** Every extended stage (lexer, parser, ast,
  check, ir, backend, repl) gets `#[cfg(test)] mod tests` with a happy path plus at
  least one error/edge case, named `thing_condition_expected`. Every exit criterion is
  a golden. Diagnostics are behaviour: each negative asserts the salient message
  substrings **and the type names**, not merely that it failed.
- **NF5 — No premature abstraction.** Follow the Slice-2 shape (table + special-case
  operator/conversion rules). No general numeric-promotion machinery, no polymorphism,
  no new modules unless the growth-structure signals in CLAUDE.md actually fire.

---

## Observable success criteria

Map 1:1 to the brief's eight exit criteria. Each is a golden (native binary or REPL
session) unless noted.

- **S1 (E1).** `f32` and `f64` are usable in effect comments, arithmetic, comparison,
  and conversion; integer/`bool` behaviour is unchanged; all prior goldens still pass.
- **S2 (E2).** Float literals parse (`3.14`, `1.5e-3`), default to `f64`; `3.` and
  `.5` are rejected so they cannot collide with the `.` print word (lexer unit tests).
- **S3 (E3).** Float arithmetic runs correctly in a native binary: `+ - *` on both
  widths, and `/` — including that `1.0 0.0 /` yields inf and `0.0 0.0 /` yields NaN
  with no trap, and NaN is detectable via `x = x`.
- **S4 (E4).** IEEE comparison is correct in a run: an ordered comparison gives the
  expected boolean, and a comparison involving NaN is false (e.g. `x = x` is false for
  a NaN produced by `0.0 0.0 /`).
- **S5 (E5).** Conversions run width-correct in a native binary: int→float,
  float→float (both directions), and float→int truncating toward zero (`3.9 >i64 .`
  prints `3`, `-3.9 >i64 .` prints `-3`).
- **S6 (E6).** A carried float survives a REPL line boundary correctly (float
  marshalling), and displays as its float value, not a bit pattern.
- **S7 (E7).** Homogeneous-operand enforcement is a sharp error: the six diagnostics
  X1–X6, each asserting the right message text and the type names.
- **S8 (E8) — dogfood.** `examples/mean.sth` computes the mean of two integer inputs
  as an `f64` (converting via `>f64`, dividing with float `/`) and prints it with
  `f.`, exercising int→float conversion, float division, and float printing in one
  honest program; compiled to a native binary it prints a known value (mean of `10`
  and `4` prints `2.5`), and it is runnable in the REPL. Plus the headline negative
  golden (mixed int/float arithmetic, X1).

---

## Scope and boundaries

**In:** `f32`/`f64`; float literals (`<digits>.<digits>` + optional exponent, default
`f64`); homogeneous `+ - *` over same-numeric-type; float-only `/`; integer-only
`mod` (unchanged); IEEE ordered `= < >` (exact `=`); `f.` print `( f64 -- )`;
numeric-generalised conversions (`>f32`/`>f64`, and float sources for `>iN`/`>uN`),
with int→float / float→float / float→int-truncate semantics; float-width-carrying IR
with backend-derived `s`/`d` register class; REPL carried-float marshalling and float
display; the six diagnostics; `examples/mean.sth` dogfood and goldens.

**Out of scope (mirrors the brief exactly — deferred, do not build):** epsilon /
approximate comparison (stdlib); an `isnan`/`isinf` primitive (NaN is user-detectable
via `x = x`; `isinf` waits for the stdlib); float `mod`/`fmod`; FP trapping or NaN/inf
static rejection; rounding-mode control; shortest-round-trip (Ryu/Grisu) printing;
checked or saturating float→int conversion; a **total order** over floats for a generic
`sort`/`max` (no generic `Ord` bound yet; revisit when generics land in **Phase 4**,
Rust-`total_cmp` style). Also still out: `i128`/`u128`; bitwise operators; the `*/`
widening primitive; structs/records; enums/ADTs and `match`; fixed-size arrays;
optional and non-null pointer types; the `Copy`/affine marker; polymorphic
operator/shuffle *signatures* (Phase 4); any heap or move semantics (Phase 3).

---

## Advisory solution approach

Not binding, but the intended shape (mirrors how Slice 2 landed).

- **Frontend (R1–R5).** Add `Type::Float { bits }` and a `FLOAT_TYPES` table beside
  `INT_TYPES` in `ast.rs`; grow `from_name`/`name`/`Display` and add
  `is_float`/`is_numeric`. Add `is_float_literal(&str)` in `lexer.rs` (digits, `.`,
  digits, optional `[eE][+-]?digits`) and a `Token::Float(f64)`, tried in the
  word-accumulation branch *before* `is_int_literal` falls through to `Word`. Add
  `TermKind::FloatLit(f64)` and route `Token::Float` in `parser.rs::parse_term`.
- **Checker (R6–R12).** In `check_operator`, split the current
  `"+" | "-" | "*" | "mod"` arm: `+ - *` require `a == b && a.is_numeric()`; add a `/`
  case requiring `a == b && a.is_float()`; keep `mod` requiring `a == b && a.is_int()`.
  Generalise the comparison arm's guard from `is_int` to `is_numeric`. In the
  conversion arm, accept a numeric target (`is_int() || is_float()`) and require a
  numeric source. Add distinct diagnostic helpers (or generalise the existing
  `operand_pair_mismatch_error`/`conversion_source_error` wording) so X1–X5 name both
  operand types / the source type and state the specific rule (`/` needs floats, `mod`
  needs integers, conversion source must be numeric). Add `f.` to `builtin_table`.
  Type a `FloatLit` as `Float { bits: 64 }` in `check_term`.
- **IR + backend (R13–R18).** Add `IrType::Float { bits }` and map it in `ir_type_of`.
  Add a float const path (`Instr::ConstF(Value, f64)`) and a `BinOp::Div`. In
  `lower_call`, route `/` to `BinOp::Div` (result carries the float operand type), and
  the existing `+ - *` arm already carries the operand type through — it works for
  floats unchanged. In `qbe.rs`, extend `width()` (`Float{32}→s`, `Float{64}→d`);
  emit `div` for `BinOp::Div`; in `Cmp`, when the operand is `Float`, emit the
  `…s`/`…d` ordered float compares; in `Conv`, dispatch on
  int/float source/target to the QBE conversion ops (R18), reusing the existing
  int-widen/narrow-canonicalize as sub-steps for sub-word integer endpoints. Emit the
  float constant as `={s|d} copy {s_|d_}{value}` (Rust's `f64` `Display` gives a
  round-trippable text QBE parses).
- **Print + REPL (R19–R21).** Add a float format `data` string and emit `f.` as a
  `printf` with a `d` argument. Make `Instr::Load`/`Store` width-aware in the backend
  (pick `loadd`/`stored`/`loads`/`stores` by the value's `IrType`) instead of always
  `loadl`/`storel`; `lower_line`'s prologue/epilogue then loads/stores a float slot
  with the float op selected from the carried `Type`. Thread the per-slot `Type` into
  the stack display so a float slot renders via `from_bits`.
- **Dogfood + goldens (S1–S8).** Add `examples/mean.sth`; add positive goldens
  (arithmetic on both widths, inf/NaN, comparison incl. NaN, all conversion cells,
  float→int truncation, the mean binary), the six negative diagnostic goldens, and a
  REPL carried-float session; confirm all prior goldens stay green.

---

## Codebase map (anchored, verified against `main`)

Paths and line numbers confirmed by reading the current source.

- **`src/ast.rs`** — `Type` enum (`Int(IntType)`, `Bool`) at `ast.rs:52`; `IntType`
  (private `bits`/`signed`) at `ast.rs:62`; `INT_TYPES` table at `ast.rs:69`;
  `Type::from_name` at `ast.rs:89`, `is_int` at `ast.rs:105`, `name` at `ast.rs:109`;
  `Display for Type` at `ast.rs:133`; `TermKind` (`IntLit`/`BoolLit`/`Call`/`If`) at
  `ast.rs:146`. → add `Type::Float { bits }` + `FLOAT_TYPES`, grow
  `from_name`/`name`/`Display`, add `is_float`/`is_numeric`, add
  `TermKind::FloatLit(f64)`.
- **`src/lexer.rs`** — `Token` enum at `lexer.rs:6` (`Int(i64)`, `Word(String)`);
  `is_int_literal` at `lexer.rs:20`; the int-vs-word decision at `lexer.rs:78`. → add
  `Token::Float(f64)` + `is_float_literal`, tried before the int/word fallthrough
  (digits both sides).
- **`src/parser.rs`** — `parse_term` at `parser.rs:235` (matches `Token::Int`,
  `true`/`false`, `if`, `then`/`else`, `Word`); `resolve_type` at `parser.rs:180`
  (uses `Type::from_name`, emits `error: unknown type`). → route `Token::Float` to
  `TermKind::FloatLit`.
- **`src/check.rs`** — `builtin_table` (`.` only, `( i64 -- )`) at `check.rs:38`;
  `operand_pair_mismatch_error` at `check.rs:199`; `conversion_source_error` at
  `check.rs:212`; `conversion_unknown_type_error` at `check.rs:225`; `check_term`
  literal typing (`IntLit`→`I64` at `check.rs:279`, `BoolLit`→`Bool`); `check_operator`
  at `check.rs:354` — arithmetic arm `"+"|"-"|"*"|"mod"` at `check.rs:362`, comparison
  arm `"="|"<"|">"` at `check.rs:374`, conversion arm at `check.rs:387`. → split `/`
  out of arithmetic (float-only), generalise `+ - *`/comparison to numeric, keep `mod`
  int-only, grow conversion target/source to numeric, add `f.` builtin, type
  `FloatLit`.
- **`src/ir.rs`** — `IrType` (`Int`/`Bool`/`Ptr`) at `ir.rs:31`; `ir_type_of` at
  `ir.rs:56`; `Instr` at `ir.rs:80` (`Const(Value,i64)` at `ir.rs:81`, `Conv(Value,Value)`
  at `ir.rs:97`); `BinOp` at `ir.rs:101`; `CmpOp` at `ir.rs:109`; `lower_line` at
  `ir.rs:174` (prologue load+relabel loop at `ir.rs:191`, epilogue store at `ir.rs:214`);
  `lower_call` at `ir.rs:377` (arithmetic arm `"+"|"-"|"*"|"mod"` at `ir.rs:406`,
  comparison at `ir.rs:422`, `.` print at `ir.rs:434`, conversion recogniser at
  `ir.rs:443`). → add `IrType::Float`, `Instr::ConstF`, `BinOp::Div`; route `/` and
  float literal; make `lower_line` load/store a float slot with the float op.
- **`src/backend/qbe.rs`** — `emit` fmt data string at `qbe.rs:13`; `width()` at
  `qbe.rs:45`; `sub_word` at `qbe.rs:62`; `emit_canonicalize` at `qbe.rs:75`;
  `emit_conv` at `qbe.rs:109`; `emit_instr` at `qbe.rs:189` (`Const` at `qbe.rs:191`,
  `Bin` at `qbe.rs:195`, `Cmp` at `qbe.rs:216`, `Print` at `qbe.rs:254`, `Load`
  `loadl` at `qbe.rs:258`, `Store` `storel` at `qbe.rs:259`). → `width()` gains
  `s`/`d`; `Bin` emits `div`; `Cmp` emits float compares; `Conv` gains the int↔float /
  float↔float ops; `ConstF` emits a QBE float constant; `Load`/`Store` become
  width-aware; add a float `printf` format for `f.`.
- **`src/driver.rs`** — pipeline wiring (`build`/`compile_so`); the print path is the
  QBE `printf`/`data $fmt` in `qbe.rs`, not a separate runtime file. → no change beyond
  what the backend needs.
- **`src/repl.rs`** — `format_stack` at `repl.rs:151` (renders each slot as `i64`);
  `Session.types` (per-slot `Type`) at `repl.rs:167`; `eval_expr` carried-stack
  marshalling at `repl.rs:247`. → thread `types` into display (R21); the float
  load/store falls out of R20's backend change.
- **`examples/`** — `gcd.sth`, `factorial.sth`, `lerp.sth`, `sign.sth`,
  `bool_abi.sth`, `rgb.sth` (unchanged). → add `examples/mean.sth`.
- **`tests/phase0.rs`**, **`tests/phase1.rs`** — existing goldens (unchanged, stay
  green). → add float positive/negative goldens and a REPL carried-float session,
  following the existing helpers (`run_and_capture_stdout`, `run_session`).

---

## Open questions and risks

Led by the genuinely hard/uncertain areas.

- **RISK 1 — NaN/inf are runtime values Sooth's compile-error lever cannot reach
  (D4), and IEEE comparison must be exactly ordered.** This is the headline risk and
  the one axis where the language's "turn silent failures into sharp compile errors"
  premise structurally does not apply: `1.0 0.0 /` = inf and `0.0 0.0 /` = NaN with no
  static prevention short of dependent types (out of scope). The correctness burden
  moves entirely onto codegen: `<`/`>`/`=` must lower to QBE's **ordered** float
  compares so any comparison against NaN is false (including `NaN = NaN`), which is
  what makes `x = x` a valid NaN test and closes the loop with D4. Mitigation: goldens
  that *run a native binary* and assert inf/NaN propagation and the false NaN compare
  (S3, S4), not just IL-level mnemonic checks; document the propagation loudly in the
  example and in `check.rs` where `=` is generalised.
- **RISK 2 — the conversion matrix is the largest hard surface (R18).** Nine-plus
  cells (int→float by source width/signedness, float→float both directions,
  float→int by source width, with sub-word integer endpoints threading through the
  existing widen/narrow-canonicalize). The existing `emit_conv` assumes integer
  source *and* target; generalising it without regressing the integer cells is the
  main codegen work. Mitigation: keep the integer path exactly as-is and dispatch to
  it as a sub-step; add a per-cell hand-built-IL unit test (as `emit_conv_il` already
  does for integers) for each new cell, plus running goldens for float→int truncation
  (S5). Truncation toward zero and out-of-range/NaN→int being unspecified are locked
  (D7); do not add checked/saturating handling.
- **RISK 3 — float literal lexing vs the `.` print word (D5).** Today `.` lexes as
  `Word(".")` and `3.14` lexes as a single `Word("3.14")` (`.` is not a lexer
  delimiter). The float-literal recogniser must require digits on **both** sides so
  `3.` / `.5` are *not* floats (they fall through to `Word` and error as unknown
  words), while `3.14`/`1.5e-3` are. The subtle failure is accidentally accepting `3.`
  as `3.0`, which would let a float literal swallow a following `.`-print in some
  spacings. Mitigation: unit tests that `3.14`/`1.5e-3` lex as `Token::Float`, that
  `3.` and `.5` do **not**, and that a plain integer still lexes as `Token::Int`;
  confirm `5 .` still lexes as `Int(5)`, `Word(".")`.
- **RISK 4 — REPL carried-float marshalling and display (R20, R21).** The carried
  buffer is `Vec<i64>` with 8-byte slots. A float slot must be stored/loaded with the
  **float** store/load (`stored`/`loadd`, `stores`/`loads`), not the integer
  `storel`/`loadl`, or the bits round-trip but the value re-enters as a stale `i64`
  and every subsequent float op reads garbage. This requires making `Instr::Load`/
  `Store` width-aware in the backend (they are hard-coded to `loadl`/`storel` today,
  at `qbe.rs:234`–`235`) and selecting the float op from the carried `Type` in
  `lower_line`. Display is the second half: a carried float shown as its `i64` bit
  pattern is meaningless (worse than Slice 2's unsigned-display caveat), so
  `format_stack` must reinterpret a float slot via `from_bits`. An `f32` in an 8-byte
  slot stores 4 bytes (`stores`) and loads 4 (`loads`); store/load must agree on width
  per slot. Mitigation: a REPL golden that leaves a float on the carried stack and
  uses it on the next line (S6), asserting both the used value and the displayed value.
- **Q1 — float-const IR representation.** `Instr::Const` carries an `i64`
  bit-payload; a float literal needs its `f64` value to emit a QBE `d_`/`s_` constant.
  **Resolution (advisory):** add `Instr::ConstF(Value, f64)` rather than reinterpreting
  the `i64` payload, so the backend never guesses whether a `Const` is integer or
  float. The implementer may instead widen `Const`; either is fine so long as the
  backend can tell the two apart. Decide and justify in the phase that lands R14.
- **Q2 — `f32` carried-slot width.** An `f32` occupies 4 of the 8 slot bytes. The
  simplest correct choice is `stores`/`loads` (4-byte) with the upper 4 bytes left
  stale, since store/load are symmetric per slot and display reads only the low 32
  bits via `f32::from_bits`. Confirm this holds across a line boundary in the S6
  golden if an `f32` is carried (the dogfood carries an `f64`; an `f32` carry is an
  extra edge test, not an exit criterion).
- **Q3 — dogfood shape (S8).** The brief fixes "mean of `10` and `4` prints `2.5`":
  `10 >f64 4 >f64 + 2.0 /` then `f.`. Keep it that honest and small (int→float
  conversion + float `/` + `f.` in one program); do not add helpers or generalise to
  N inputs (no arrays this slice).

---

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Frontend Type + float-literal lexing (R1–R4). Add `Type::Float { bits }` and a `FLOAT_TYPES` table beside `INT_TYPES` in ast.rs; grow `from_name`/`name`/`Display`; add `is_float`/`is_numeric`. Add `Token::Float(f64)` and `is_float_literal` in lexer.rs (digits both sides of the dot, optional exponent), tried before the int/word fallthrough so `3.14`/`1.5e-3` lex as floats and `3.`/`.5` do not. Add `TermKind::FloatLit(f64)` and route `Token::Float` in parser.rs. Unit tests: `f32`/`f64` from_name/Display round-trip and `f128`→None; `3.14`/`1.5e-3` lex as Float; `3.`/`.5` are not floats; a plain integer still lexes as Int; `5 .` still lexes as Int + Word(\".\"). Green.",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Checker operator/conversion rules (R5–R12). In check_operator: generalise `+ - *` to same-numeric-type; add `/` as float-only; keep `mod` integer-only; generalise `= < >` to same-numeric-type producing bool; grow the conversion recogniser's target set to include f32/f64 and require a numeric (not just integer) source. Add `f.` `( f64 -- )` to builtin_table beside `.`; type a FloatLit as `Float { bits: 64 }`. Add/generalise diagnostics so X1 (mixed int/float arith), X2 (mixed float-width), X3 (`/` on ints), X4 (`mod` on floats), X5 (conversion of a bool/non-numeric), X6 (unknown float target `>f128`) each name the type(s) and state the rule. Branch joins unify over float types too. Unit tests per stage (happy + error), asserting message text AND type names. Green.",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Typed IR + float codegen (R13–R17). Add `IrType::Float { bits }` and map it in ir_type_of. Add a float const path (`Instr::ConstF(Value, f64)`, resolving Q1) and `BinOp::Div`; route `/` and the float literal in lower_call/lower_term (result carries the float operand type). In qbe.rs: `width()` derives `s`/`d`; `Bin` emits `div` for `BinOp::Div` and runs `add`/`sub`/`mul` at `s`/`d` width for float results; `Cmp` emits the ordered float compares (`ceqs`/`ceqd`/`clts`/`cltd`/`cgts`/`cgtd`) when the operand IrType is Float; emit the float constant as `={s|d} copy {s_|d_}{value}`. Sub-word canonicalization stays untouched and never runs for floats. Keep the register class out of IrType (NF2). Unit tests: `Type→IrType` for f32/f64; float arith/`/` emit float ops at `s`/`d`; a float compare emits the ordered float mnemonic; a NaN compare lowers to an ordered compare (false against NaN). Green.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Conversion-op lowering, the full numeric matrix (R18). Extend emit_conv (integer-only today) to dispatch on int/float source+target: int→float (`swtof`/`sltof`/`uwtof`/`ultof` by source width/signedness, sub-word source widened first); float→float (`exts` up, `truncd` down); float→int truncating toward zero (`stosi`/`dtosi`, then the existing narrow/canonicalize for a sub-word target). Keep the integer→integer path exactly as-is and reuse it as a sub-step; float endpoints never touch sub-word canonicalization. Out-of-range/NaN→int stays unspecified (D7); no checked/saturating handling. Per-cell hand-built-IL unit tests (as emit_conv_il does for integers) for each new cell, plus a running golden for float→int truncation (`3.9 >i64 .`→`3`, `-3.9 >i64 .`→`-3`). No regression to the integer conversion goldens. Green.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "`f.` printing + REPL carried float (R19–R21, RISK 4). Add a float `printf` format (`%g\\n`) and emit `f.` as a printf passing the value as `d`. Make `Instr::Load`/`Store` width-aware in qbe.rs (`loadd`/`stored`/`loads`/`stores` by the value's IrType instead of hard-coded `loadl`/`storel`); `lower_line`'s prologue/epilogue then loads/stores a float slot with the float op selected from the carried `Type` (a float slot needs no integer Conv-relabel). Thread `Session.types` into the stack display so `format_stack` renders an `f64` slot via `f64::from_bits` (an `f32` slot via `f32::from_bits` on the low 32 bits); integer/bool slots unchanged. Confirm the `f32`-in-8-byte-slot choice (Q2). Unit tests: `f.` emits the float print; a float slot round-trips its store/load; display renders a float value not a bit pattern. REPL golden: leave a float on the carried stack and use + display it on the next line. Green.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 6,
      "focus": "Dogfood + goldens (S1–S8). Add `examples/mean.sth` (mean of two ints as f64: `>f64` conversion, float `/`, `f.`; mean of 10 and 4 prints `2.5`), compiled to a native binary and runnable in the REPL. Positive goldens: `+ - *` on both widths; `/` producing inf (`1.0 0.0 /`) and NaN (`0.0 0.0 /`) with NaN detected via `x = x`; an ordered comparison and a NaN comparison; int→float, float→float both directions, float→int truncation; the mean binary. Negative goldens: the six diagnostics X1–X6 asserting message text AND type names, with the mixed int/float arithmetic (X1) as the headline. A REPL carried-float session. Confirm all Phase 0/1/Slice-1/Slice-2 goldens stay green (NF3). Green.",
      "effort": "M",
      "difficulty": "standard"
    }
  ]
}
```
