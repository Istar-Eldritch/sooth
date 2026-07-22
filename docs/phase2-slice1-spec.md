# Phase 2 Slice 1 — technical specification

**The typed-core spine: a `Type`-carrying checker with two concrete types (`i64`, `bool`).**

Read alongside [phase2-slice1-brief.md](./phase2-slice1-brief.md), [../DESIGN.md](../DESIGN.md),
[../ROADMAP.md](../ROADMAP.md), [../CLAUDE.md](../CLAUDE.md). This spec is scoped to Slice 1 only
and honours the brief's locked decisions, exit criteria, and out-of-scope list exactly. It builds
on the Phase 0/1 compiler on `main` (lexer → parser → checker → IR → QBE emit → driver / REPL).

Craft-project discipline (CLAUDE.md): this is deliberately small. No new abstraction beyond what
the two-type checker needs. Where a table or a match arm suffices, do not build a trait.

---

## 1. Goal

Replace the arity-only checker with one that carries a concrete `Type` per virtual-stack slot and
unifies **type** (not just depth) through each word body and at `if` join points. Introduce exactly
two frontend types, `i64` and `bool`, so that a type mismatch becomes possible and the spine is
provable. Nothing else from the Phase 2 epic (numeric tower breadth, structs, enums/match, arrays,
optional/pointer, the `Copy` marker, polymorphic shuffle signatures) is in this slice.

---

## 2. Locked decisions (from the brief, restated for traceability)

- **D1.** `int` is hard-renamed to `i64`. No `int` alias. Slot types are concrete from here on.
- **D2.** `true` / `false` are `bool` literals: a new literal term kind, analogous to the integer
  literal.
- **D3.** `bool` is a distinct IR type (`IrType::Bool`), lowering to QBE `w` (0/1). Not frontend-only
  sugar over `i64`; the type flows through the IR to the backend.
- **D4.** `.` (print) stays `( i64 -- )`. No `bool` printing in Slice 1.
- **D5.** Stack shuffles (`dup`/`drop`/`swap`/`over`/`rot`) stay **structural and type-transparent**:
  they move whatever concrete slot types are present (so `dup` of a `bool` yields two `bool`s). They
  are **not** table entries with fixed types and **not** polymorphic signatures (that is Phase 4).
- **D6.** Frontend `Type` and backend `IrType` stay distinct. `Type = { I64, Bool }`; `IrType` gains
  `Bool` alongside `Int`/`Ptr`. Lowering maps `Type → IrType`. `bool` is never collapsed into `Int`
  in the IR.

Typed builtin signatures (replacing the arity table):

| word            | effect                | notes |
|-----------------|-----------------------|-------|
| `+ - * mod`     | `( i64 i64 -- i64 )`  | |
| `= < >`         | `( i64 i64 -- bool )` | result IR type is `Bool`; QBE `c*` already yields a `w` 0/1 |
| `.`             | `( i64 -- )`          | D4 |
| shuffles        | structural (D5)       | not table entries |

---

## 3. Requirements (numbered, traceable)

Each requirement traces to one or more exit criteria E1–E6 (§5).

**R1 — `int` → `i64` rename (E1).** Every occurrence of the type name `int` in surface source becomes
`i64`: examples (`gcd.sth`, `factorial.sth`, `lerp.sth`) and every golden / unit-test source string
that spells `int`. No `int` alias is accepted; a program using `int` is now an unknown-type error (R6).

**R2 — frontend `Type` enum (E1, E2).** A frontend `Type { I64, Bool }` lands in `ast.rs`. `TypedSlot.ty`
changes from `String` to `Type`. The type name in an effect comment is resolved to a `Type` when the
`TypedSlot` is built.

**R3 — `bool` literals (E2).** `true` and `false` lex and parse to a new `TermKind::BoolLit(bool)`.
They are ordinary literal terms (like `IntLit`), usable anywhere a term is.

**R4 — `bool` is a real IR type through the backend (E2).** `IrType` gains `Bool`. Lowering tags
`BoolLit` results and comparison (`= < >`) results as `Bool`; arithmetic and integer literals stay
`Int`. The backend emits `w` for `Bool`-typed values and `l` for `Int`, inserting a `w → l` extension
where a `bool` value fills an 8-byte sink (the line-wrapper `storel`, and any `l`-typed return). The
8-byte buffer marshalling keeps its size (bool and i64 are both one 8-byte slot); only the carried
type label changes.

**R5 — typed checker: per-slot `Type` simulation (E3).** The checker simulates a stack of concrete
`Type`s (not a depth counter). Builtins come from a typed-effect table (§2). Every operand is checked
against its expected type; a mismatch is a sharp, located error naming both types (e.g.
`` expected `i64`, found `bool` ``).

**R6 — unknown type name (E3).** A type name in an effect comment that is not `i64` or `bool` is a
reported error naming the offending name (e.g. `` unknown type `foo` ``).

**R7 — `if` requires `bool` (E2, E3).** `if` consumes a `bool` on top for its condition. A non-`bool`
condition (e.g. `5 if …`) is an error naming the expected/found types at the `if`.

**R8 — declared-output type check (E3).** A word body must leave exactly the declared **output types**
(right count *and* per-slot type). A mismatch against the effect comment is an error citing the
declared effect.

**R9 — structural, type-transparent shuffles (D5, E3).** `dup`/`drop`/`swap`/`over`/`rot` move the
concrete slot types present, in the checker, with no fixed or polymorphic signature. `dup` of a `bool`
yields two `bool`s; `swap` of `( i64 bool )` yields `( bool i64 )`; etc.

**R10 — branch-join type unification (E4).** At an `if` join, the `then` and `else` arms must agree on
**both depth and per-slot type**. A disagreement is an error naming the differing slot types. (Depth
disagreement keeps its existing message.)

**R11 — REPL carries types (E5).** `infer_line` simulates from, and returns, a typed carried stack.
The `Session` records a `Type` per carried slot. A bare line that type-errors reports it and the
session survives (no state mutation on error, as today). The 8-byte buffer and `format_stack` display
are unchanged (D4: no bool printing; a `bool` on the stack displays as its `0`/`1` slot value).

**R12 — dogfood + goldens (E1, E5, E6).** `gcd`/`factorial`/`lerp` still produce `5`/`120`/`30` as
`i64`. A new `bool`-branching program (the `sign` dogfood) compiles to a native binary, runs, and is
runnable in the REPL. At least one negative golden proves a type error is a sharp diagnostic; the five
diagnostics of §4 are covered as negative goldens/units asserting the salient substrings and type names.

---

## 4. Diagnostics (behaviour, asserted on message content)

Exact wording is the implementer's; tests assert the salient substrings and the type names, in the
existing diagnostic style (see `check.rs` `underflow_error` / `branch_mismatch_error`).

1. **`if` condition not bool** — `5 if 1 else 2 then` → mentions `` expected `bool` `` / `` found `i64` `` (R7).
2. **Operand type mismatch** — a `bool` where `+` wants `i64` → `` expected `i64` `` / `` found `bool` `` (R5).
3. **Branch join type mismatch** — a `then` arm leaving `i64`, an `else` arm leaving `bool` → the arms
   disagree, naming the slot types (R10).
4. **Declared-output type mismatch** — a word declared `( i64 -- bool )` whose body leaves `i64` → the
   mismatch against the declared effect (R8).
5. **Unknown type name** — an effect comment naming an unknown type → `` unknown type `foo` `` (R6).

---

## 5. Success criteria (the brief's six exit criteria, observable)

- **E1.** `int` renamed to `i64` throughout; `gcd`/`factorial`/`lerp` compile as `i64` and still print
  `5`/`120`/`30` (Phase 0 goldens updated, green).
- **E2.** `bool` is a real, distinct type: `true`/`false` literals parse; comparisons produce `bool`;
  `if` requires a `bool`.
- **E3.** Type mismatches are sharp compile errors (the five diagnostics of §4).
- **E4.** Branch join points unify on **type**, not just depth.
- **E5.** The Phase 1 REPL still works; its carried stack tracks types; a bare line that would
  type-error reports it and the session survives (Phase 1 goldens green, updated for the `int`→`i64`
  spelling).
- **E6.** Dogfood: `: sign ( i64 -- i64 ) 0 > if 1 else 0 then ;` compiled to a native binary (and
  runnable in the REPL), plus at least one negative golden proving a type error is a sharp diagnostic.

"Green" (CLAUDE.md): `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

---

## 6. Codebase map (verified against `main`)

Anchors are `path:line` at spec time; verify before editing (line numbers drift).

- **`src/ast.rs`**
  - `TypedSlot` (`ast.rs:42`) — `{ name: Option<String>, ty: String }`. `ty` → `Type` (R2).
  - `TermKind` (`ast.rs:54`) — `IntLit(i64)` / `Call(String)` / `If{…}`. Add `BoolLit(bool)` (R3).
  - Lowest common ancestor of parser + checker + ir → the `Type` enum lands here (R2, per brief).
- **`src/lexer.rs`**
  - `Token` (`lexer.rs:6`), `is_int_literal` (`lexer.rs:20`), classification at `lexer.rs:78`.
    `true`/`false` need no new token: they lex as `Token::Word` and are recognised in the parser
    (like `if`/`then`/`else`). No lexer change is strictly required for R3; keep it in the parser.
- **`src/parser.rs`**
  - `parse_term` (`parser.rs` ~200) — matches `Token::Int` → `IntLit`, word `if` → `If`, else `Call`.
    Add `true`/`false` word arms → `BoolLit` (R3).
  - `parse_slot` (`parser.rs` ~155) — builds `TypedSlot { ty: <word> }`. Resolve the type-name word to
    `Type` here; unknown name → error (R6). This keeps type resolution at the parse boundary and the
    checker clean.
- **`src/check.rs`** (the hard core)
  - `pub type Arity = (usize, usize)` (`check.rs:13`) and `builtin_table()` (`check.rs:16`) — replaced
    by a typed word-signature representation and a typed-effect table (R5).
  - `check` (`check.rs:57`), `check_def` (`check.rs:74`), `check_word` (`check.rs:104`),
    `infer_line` (`check.rs:86`), shared `check_terms` (`check.rs:172`) / `check_term` (`check.rs:184`),
    `Ctx::Word`/`Ctx::Line` (`check.rs:38`). Depth simulation becomes typed-stack simulation (R5–R10).
  - `effect_str` (`check.rs:96`) — reads `slot.ty` as `String`; switch to `Type` display.
  - `branch_mismatch_error` (`check.rs:154`) — extend/duplicate for the type-mismatch-at-join case (R10).
  - Module doc says "Arity only for now; type unification is a later ROADMAP phase" — this slice is
    that phase; update the doc.
- **`src/ir.rs`**
  - `IrType { Int, Ptr }` (`ir.rs:29`) — add `Bool` (R4).
  - `lower_word` (`ir.rs`) — `params`/`ret` hard-coded `IrType::Int` (`ir.rs:188`, `ir.rs:192`); map
    declared slot `Type → IrType` (R4).
  - `FuncBuilder` (`ir.rs` ~230) + `lower_call` (`ir.rs:300`) / `lower_term` (`ir.rs:287`) — add a
    per-`Value` `IrType` map: `IntLit`→Int, `BoolLit`→Bool, `Cmp`→Bool, `Bin`→Int, `Load`→Int (buffer
    slot is 8-byte i64), `Phi`→arms' unified type, `Call` ret→word output type, params→declared type.
  - `lower_if` (`ir.rs:376`) — phi at join; arm value types are guaranteed equal by the checker (R10).
  - `lower_line` (`ir.rs:127`) — the wrapper's `storel` epilogue is a `w → l` sink for a `bool` top (R4).
  - `ir::Arity` (`ir.rs:89`) is independent of the checker's env; ir keeps needing only arities.
- **`src/backend/qbe.rs`**
  - `emit_instr` (`qbe.rs:62`) — currently emits `l` uniformly. Choose width from the value's `IrType`:
    `Bool`→`w`, `Int`→`l`; extend `w→l` at `storel`/`l`-return sinks (R4). `Cmp` already emits a `c*`
    that yields a 0/1.
  - `emit_func` (`qbe.rs`) — `params`/`ret` width from `IrType` (`l`/`w`).
- **`src/repl.rs`**
  - `WordEntry` (`repl.rs:95`) carries `arity: Arity`; add the word's typed effect (R11).
  - `arity_env` (`repl.rs` ~168) → build a typed env for the checker; derive an arity map for
    `ir::lower_line`/`lower_word` from it.
  - `eval_def` (`repl.rs:189`) calls `check::check_def`; `eval_expr` (`repl.rs:230`) calls
    `check::infer_line` and `ir::lower_line` with the *same* env today — split into typed (checker) +
    arity (ir) (R11). `format_stack` (`repl.rs:140`) and `buf: Vec<i64>` unchanged (R11, D4).
- **`examples/gcd.sth`, `factorial.sth`, `lerp.sth`** — `int` → `i64` (R1). New `examples/sign.sth` (R12).
- **`tests/phase0.rs`** — golden run assertions + the `( int -- int )` diagnostic substrings → `i64`;
  add the `sign` binary golden and the negative type-error golden (R1, R12).
- **`tests/phase1.rs`** — REPL session goldens use `( int -- int )` in def lines → `i64`; add a
  type-erroring line that the session survives (R5, R11).

---

## 7. Open questions / risks

- **RK1 (backend width, the sharpest).** QBE is strongly typed: a `w` value used where `l` is expected
  (e.g. `storel`, an `l` return, an `l` call arg) is a type error in the QBE IL. Slice 1 keeps buffer
  slots and the C-ABI at 8 bytes, so a `bool` (`w`) that reaches such a sink must be extended
  (`%t =l extuw %b`). The only `bool` sinks in-slice are: the line-wrapper `storel` epilogue, and a
  word whose declared output is `bool` returning across the `l`-slot ABI. `if` consumes `bool` via
  `jnz`, which accepts any width, so branching needs no extension. **Recommendation:** track `IrType`
  per value in the IR (§6), pick widths in the backend from it, and insert the `w→l` extension at
  `l`-typed sinks only. This is the one place to test carefully (a `5 3 >` line stores a `bool`).
- **RK2 (checker env vs ir env).** `check::infer_line`/`check_def` currently share one
  `HashMap<String, Arity>` with `ir::lower_line`/`lower_word` in `repl.rs`. The typed checker needs a
  typed signature per word; ir still needs only arities. Do not unify them into one type — build the
  typed env for the checker and derive the arity map for ir. Keep `ir::Arity` as is.
- **RK3 (where type resolution lives).** This spec resolves type names → `Type` in `parse_slot` (parse
  boundary), so the unknown-type error (R6) is a parse-time diagnostic and the checker only ever sees
  resolved `Type`s. Alternative: resolve in the checker. Parse-time is simpler and keeps the checker
  operating purely on `Type`; chosen unless a growth signal argues otherwise.
- **RK4 (`bool` on the carried stack display).** `format_stack` prints slots as `i64`. A `bool` slot
  displays as `0`/`1`. D4 forbids bool *printing* via `.`; the residual-stack display is a driver
  artifact and is left unchanged. Flagged so it is a conscious choice, not an oversight.
- **RK5 (scope creep).** The numeric tower, `*/` widening, literal-default adoption, structs, enums,
  `match`, arrays, optional/pointer, the `Copy` marker, and polymorphic shuffle signatures are **out of
  scope** (later slices / Phase 4). Resist adding a third type "while we're here".

---

## 8. Phased delivery plan

Five phases. Each is independently green (`cargo fmt --check && cargo clippy -- -D warnings &&
cargo test`) and adds unit coverage beside the stage it touches (CLAUDE.md). Signature-changing edits
include all their call sites in the same phase so the tree always compiles. Phases 3 and 4 are the hard
type-reasoning core; 1, 2, 5 are mechanical.

**Phase 1 — `i64` rename + frontend `Type` (standard).** Add `Type { I64, Bool }` to `ast.rs`; change
`TypedSlot.ty: String → Type`; resolve the type-name word to `Type` in `parse_slot`, unknown → error
(R6); update `effect_str` and any `slot.ty` readers to the `Type` display. Rename `int`→`i64` in the
three examples and in every golden/unit-test source string (`tests/phase0.rs`, `tests/phase1.rs`, and
the inline `check.rs`/`ir.rs`/`parser.rs`/`qbe.rs`/`repl.rs` test sources). Checker and IR behaviour are
otherwise unchanged (still depth / `Int`). Green.

**Phase 2 — `bool` literals + `bool` through the backend (standard).** Parser: `true`/`false` word arms
→ `TermKind::BoolLit(bool)`. `ir.rs`: add `IrType::Bool`; lower `BoolLit` to a const tagged `Bool`; tag
`Cmp` results `Bool`; add the per-`Value` `IrType` map; map declared slot `Type→IrType` for
`lower_word` params/ret. `qbe.rs`: emit `w` for `Bool`, `l` for `Int`, with the `w→l` extension at
`storel`/`l`-return sinks (RK1). Checker still arity/depth (a `bool` literal is just depth+1). Green.

**Phase 3 — typed checker core (hard).** Replace `Arity`/`builtin_table()` with a typed word-signature
representation and a typed-effect table (§2). Simulate a stack of `Type` in `check_terms`/`check_term`:
operand type checks (R5), `if`-requires-`bool` (R7), declared-output per-slot type check (R8),
structural type-transparent shuffles (R9). `infer_line` takes and returns a typed carried stack. Update
`repl.rs`: `WordEntry` carries the typed effect; build the typed checker env and derive the arity map
for ir (RK2); `eval_def`/`eval_expr` rewired; `Session` records a `Type` per carried slot (R11). Branch
join still checks depth only (upgraded in Phase 4). Green.

**Phase 4 — branch-join type unification (hard).** Upgrade the `if` join in `check_term` to unify
`then`/`else` arms on **per-slot type** as well as depth; mismatch names the differing types (R10). The
IR `lower_if` phi is unaffected in shape (arm value types are now guaranteed equal by the checker).
Green.

**Phase 5 — dogfood + goldens (standard).** Add `examples/sign.sth`
(`: sign ( i64 -- i64 ) 0 > if 1 else 0 then ;` with a `main` that prints a `sign` result). In
`tests/phase0.rs`: assert `sign` compiles to a binary and runs; add the five negative diagnostics (§4)
as golden assertions on message substrings + type names. In `tests/phase1.rs`: add a REPL line that
type-errors and prove the session survives (existing session goldens already spell `i64` from Phase 1).
Confirm `gcd`/`factorial`/`lerp` still print `5`/`120`/`30`. Green — all six exit criteria met.

---

## Phases (JSON)

```json
{
  "phases": [
  {
    "phase": 1,
    "focus": "i64-rename-and-type",
    "title": "i64 rename + frontend Type enum",
    "difficulty": "standard",
    "summary": "Hard-rename int->i64 across examples and all test sources; add a frontend Type { I64, Bool } enum; change TypedSlot.ty from String to Type; resolve type-name words to Type in parse_slot with an unknown-type error; update effect_str and slot.ty readers. Checker and IR behaviour otherwise unchanged (still depth / Int).",
    "changes": [
      "src/ast.rs: add `pub enum Type { I64, Bool }`; change TypedSlot.ty to Type",
      "src/parser.rs: resolve the slot type-name word to Type in parse_slot; unknown name -> reported error naming it",
      "src/check.rs: effect_str and any slot.ty reader use the Type display",
      "examples/gcd.sth, examples/factorial.sth, examples/lerp.sth: int -> i64",
      "tests/phase0.rs, tests/phase1.rs and inline test sources in check.rs/ir.rs/parser.rs/qbe.rs/repl.rs: int -> i64 in source strings and diagnostic substrings"
    ],
    "tests": [
      "parser: `parse_slot_resolves_i64_and_bool_expected`",
      "parser: `parse_slot_unknown_type_name_is_error` (asserts `unknown type` and the name)",
      "existing parser/check/ir/qbe/repl unit tests remain green with i64 spelling"
    ],
    "exit": "cargo fmt --check && cargo clippy -- -D warnings && cargo test all green; examples and goldens spell i64; unknown type name in an effect comment is a reported error."
  },
  {
    "phase": 2,
    "focus": "bool-literals-and-ir",
    "title": "bool literals + bool through the backend",
    "difficulty": "standard",
    "summary": "Parse true/false to TermKind::BoolLit(bool); add IrType::Bool; tag BoolLit and comparison results as Bool; track IrType per Value in the IR; map declared slot Type->IrType for lower_word params/ret; emit QBE `w` for Bool and `l` for Int with a w->l extension at storel / l-return sinks. Checker still arity/depth.",
    "changes": [
      "src/ast.rs: add TermKind::BoolLit(bool)",
      "src/parser.rs: parse_term arms for the words `true`/`false` -> BoolLit",
      "src/ir.rs: add IrType::Bool; lower BoolLit to a Bool-tagged const; tag Cmp results Bool; per-Value IrType map (IntLit/Bin/Load=Int, BoolLit/Cmp=Bool, Phi=unified, Call=word-output, params=declared); lower_word maps Type->IrType for params/ret",
      "src/backend/qbe.rs: emit_instr/emit_func choose width from IrType (Bool=w, Int=l); insert w->l extension where a bool value fills a storel or l-typed return"
    ],
    "tests": [
      "parser: `parse_true_false_are_bool_literals`",
      "ir: `lower_bool_literal_is_bool_typed`",
      "ir: `lower_comparison_result_is_bool`",
      "backend: `emit_bool_value_uses_w_width`",
      "backend: `emit_comparison_line_stores_bool_via_extension` (a `5 3 >` line stores a bool to the 8-byte slot)"
    ],
    "exit": "true/false parse; comparisons produce a Bool-typed IR value lowering to QBE `w`; the line wrapper stores a bool correctly; all green."
  },
  {
    "phase": 3,
    "focus": "typed-checker-core",
    "title": "typed checker core (Type stack simulation)",
    "difficulty": "hard",
    "summary": "Replace the Arity table with a typed word-signature representation and a typed-effect builtin table. Simulate a stack of concrete Type through each body: operand type checks, if-requires-bool, declared-output per-slot type check, structural type-transparent shuffles. infer_line takes and returns a typed carried stack. Rewire repl.rs (WordEntry typed effect, typed checker env + derived arity env for ir, Session carries a Type per slot). Branch join still checks depth only.",
    "changes": [
      "src/check.rs: replace `Arity`/builtin_table with a typed effect representation and typed-effect table; check_terms/check_term simulate Vec<Type>; operand type mismatch, if-wants-bool, declared-output type, structural shuffles; infer_line signature carries typed stack; update module doc",
      "src/repl.rs: WordEntry carries the typed effect; build typed checker env and derive an arity map for ir::lower_line/lower_word; eval_def/eval_expr rewired; Session records a Type per carried slot; format_stack and buf unchanged"
    ],
    "tests": [
      "check: `check_type_propagates_through_body_expected`",
      "check: `check_if_condition_not_bool_is_error` (expected bool / found i64)",
      "check: `check_operand_type_mismatch_is_error` (expected i64 / found bool)",
      "check: `check_declared_output_type_mismatch_is_error`",
      "check: `check_shuffle_dup_bool_is_type_transparent`",
      "check: `infer_line_carries_slot_types_expected`"
    ],
    "exit": "checker simulates concrete Types; the four non-join diagnostics (if-cond, operand, declared-output, plus unknown-type from phase 1) are sharp; REPL carried stack tracks types; all green."
  },
  {
    "phase": 4,
    "focus": "branch-join-type-unification",
    "title": "branch-join type unification",
    "difficulty": "hard",
    "summary": "Upgrade the if join in check_term to unify then/else arms on per-slot type as well as depth; a disagreement is an error naming the differing slot types. The IR lower_if phi is unchanged in shape (arm value types are now guaranteed equal by the checker).",
    "changes": [
      "src/check.rs: at the if join, compare arms slot-by-slot on Type (not just depth); add/extend the join diagnostic to name the differing types"
    ],
    "tests": [
      "check: `check_branch_join_types_agree_ok`",
      "check: `check_branch_join_type_mismatch_is_error` (names i64 and bool)",
      "check: existing depth-mismatch message still asserted"
    ],
    "exit": "if join points unify on type; a type disagreement across arms is a sharp diagnostic naming the slot types; all green."
  },
  {
    "phase": 5,
    "focus": "dogfood-and-goldens",
    "title": "sign dogfood + negative goldens",
    "difficulty": "standard",
    "summary": "Add the sign dogfood example, compiled to a native binary and run; add negative goldens for the five diagnostics asserting the right message text and type names; add a REPL line that type-errors and prove the session survives; confirm gcd/factorial/lerp still print 5/120/30 as i64.",
    "changes": [
      "examples/sign.sth: `: sign ( i64 -- i64 ) 0 > if 1 else 0 then ;` plus a main that prints a sign result",
      "tests/phase0.rs: assert sign compiles to a binary and runs; add the five negative diagnostics as golden assertions on substrings + type names",
      "tests/phase1.rs: add a REPL line that type-errors and prove the session survives"
    ],
    "tests": [
      "phase0: `sign_compiles_and_runs`",
      "phase0: `if_condition_not_bool_reports_diagnostic`",
      "phase0: `operand_type_mismatch_reports_diagnostic`",
      "phase0: `branch_join_type_mismatch_reports_diagnostic`",
      "phase0: `declared_output_type_mismatch_reports_diagnostic`",
      "phase0: `unknown_type_name_reports_diagnostic`",
      "phase1: `type_error_line_reports_and_session_survives`"
    ],
    "exit": "all six exit criteria met; sign runs as a native binary and in the REPL; the five diagnostics are negative goldens; gcd/factorial/lerp print 5/120/30; all green."
  }
  ]
}
```
