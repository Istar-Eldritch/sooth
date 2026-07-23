# Phase 2 Slice 2 — Integer Tower and Explicit Conversions (condensed)

Widens the typed-core spine from two types (`i64`, `bool`) to a fixed-width integer tower, paying for the three mechanisms a tower forces: **width** (register class `w`/`l`), **signedness** (sign vs zero extend, signed vs unsigned compare), and **explicit conversion** (no implicit promotion). Craft-scoped: special-cased and table-driven, no general polymorphism/inference (that is Phase 4 of the roadmap).

## Type set

Signed `i8 i16 i32 i64`, unsigned `u8 u16 u32 u64`, plus unchanged `bool`. The eight integer types are **table-generated** from `{bits ∈ 8/16/32/64} × {signed}`, not hand-written variants. `bool` is not in the numeric tower (no arithmetic, comparison-as-int, or `>bool`).

## Requirements (as implemented)

**Frontend (R1–R3).** `Type::Int { bits: u8, signed: bool }` + `Type::Bool`, driven by a private `(name, bits, signed)` table. `from_name` resolves the nine names and rejects everything else (`i128`/`u128`/`f32` → `None`); `Display`/`name` round-trips each spelling. Parser resolves the new names in effect-comment slots for free; unknown names keep the existing `error: unknown type` diagnostic (X5).

**Literals & conversions (R4–R7).** Integer literals are always `i64`; no inference. Eight conversion words `>i8`..`>u64` each have effect `( <any int> -- <target> )`, recognised in the checker by stripping a leading `>` and resolving the rest via `from_name` (no lexer/grammar change; `>i32` is one token). Semantics: widen sign-extends if **source** signed / zero-extends if unsigned; narrow truncates (well-defined wrap, the one deliberately-unsharp edge); same-width is a bit-level relabel. Conversion of a `bool` is a type error (X4).

**Arithmetic & comparison (R8–R11).** `+ - * mod` require the **same** integer type, produce that type; `= < >` require the same type, produce `bool`. Mixed width/sign is a sharp located error naming both types (X1, X2). `<`/`>` emit signed (`cslt`/`csgt`) or unsigned (`cult`/`cugt`) compares by operand signedness; `=` is `ceq` (signedness-agnostic). No implicit promotion; `bool` operand to arithmetic stays an error. Shuffles (`dup drop swap over rot`) stay structural and type-transparent.

**Print (R12).** `.` stays fixed `( i64 -- )`; narrower/unsigned values print via `>i64 .`.

**IR & backend (R13–R15).** `IrType::Int { bits, signed }` + `Bool` + `Ptr`; `Ptr` opaque, `Bool` distinct. Register class (`w` for `bits ≤ 32`, `l` for `64`) and op signedness are **derived in the backend**, never pushed into the IR (backend stays WASM-ready). **Single sub-word canonicalization point** per dirtying op (narrowing conversion, width-overflowing arithmetic): sign-extend for signed, zero/mask for unsigned, so no two paths disagree on high bits.

**REPL (R16).** Carried stack records each slot's true `Type`. Resolution of the Slice-1 deferral: carried slots stay 8-byte and are **relabeled to their true type after load** (canonicalized on use), proven by a golden across a line boundary. Display keeps the `i64` view (no unsigned printing shipped, so the golden's expected output reflects that).

## Diagnostics (tested as behaviour, asserting message + both type names)

- **X1** mixed-width arithmetic (`1 >i32 5 +`) — headline negative.
- **X2** mixed-width/sign comparison (`u8 i8 <`).
- **X3** declared-output needs conversion (`( -- u8 ) 5`).
- **X4** conversion of a non-integer (`>i32` on a `bool`).
- **X5** unknown type name (`>i128`, unknown slot type).

## Scope

**In:** eight integer types; target-only conversions `>i8`..`>u64`; homogeneous `+ - * mod` / `= < >`; signed/unsigned compare and sign/zero-extend widen; truncating narrow; width/signedness-carrying IR; backend-derived class + canonicalization; REPL carried-type tracking; RGB dogfood + goldens.

**Out (deferred):** floats; `i128`/`u128`; bitwise ops; `*/`; integer division `/`; checked/saturating conversion; per-width/unsigned printing; structs/enums/arrays/match; optional/non-null pointers; `Copy`/affine marker; polymorphic signatures/monomorphisation; heap/move semantics.

## Non-functional

Green (`fmt --check && clippy -D warnings && test`) at every phase; invariants held (QBE-only, backend-neutral IR, opaque `Ptr`, no `w`/`l` in `IrType`, no JIT, `core` `no_std`); no regressions in Phase 0/1/Slice-1 goldens; unit coverage per stage; no abstraction beyond the table + special-case rules.

## Success criteria (S1–S6)

All eight types usable end-to-end with `bool` unchanged; conversions run width-correct in a native binary; mixed-type ops reported (X1–X5); signed vs unsigned compare of the same bit pattern give differing booleans; a sub-word value survives a REPL line boundary; and `examples/rgb.sth` packs three `u8` channels into an `i32` via homogeneous `i32` arithmetic and unpacks blue via `256 >i32 mod >u8` (no `/`), printed via `>i64 .`, plus the X1 headline and truncation golden (`511 >u8 >i64 .` → `255`).

## Delivery (5 phases, as landed)

1. **Frontend `Type` growth** (`601e4081`) — table-generated `Type::Int`+`Bool`; `from_name`/`name`/`Display`; mechanical match updates across ast/check/ir/parser.
2. **Checker operator + conversion rules** (`e49af826`) — homogeneous `+ - * mod` / `= < >` as structural special cases; `>iN`/`>uN` rule; `.` stays a fixed builtin; X1–X5 diagnostics. (ast, check, tests.)
3. **Typed IR + width/signedness-correct QBE** (`83e7a8ac`, review fix `25acbdad`) — `IrType::Int`; carry operand type/signedness; backend-derived `w`/`l` and signed/unsigned compares; single canonicalization point. (ast, ir, backend/qbe.)
4. **Conversion-op lowering** (`94d68d32`) — IR conversion instr; sign/zero-extend on widen by source signedness, truncate-and-canonicalize on narrow, copy/relabel same-width. (ir, backend/qbe.)
5. **Dogfood + goldens + REPL carried value** (`40f1ebd4`) — `examples/rgb.sth`; positive goldens (RGB binary, signed-vs-unsigned compare, `511 >u8` truncation); five negative diagnostic goldens; REPL sub-word carried-value session; carried slots relabeled to true type after load. (examples, backend/qbe, ir, repl, tests.)

### Resolved open questions

- **Q1/RISK (canonicalization):** one point per dirtying op; simple-and-correct over minimal-mask analysis.
- **Q2 (REPL carried sub-word):** path (a) — 8-byte slot, relabel/canonicalize on use.
- **Q3 (dogfood without `/`):** unpack blue only via `mod`; do not add `/`.
- **Q4 (display):** `i64` reinterpretation, acceptable (no unsigned printing shipped).
- **Q5 (same-width relabel):** `copy`/relabel only, changing `IrType` so the next op reads the new signedness.
