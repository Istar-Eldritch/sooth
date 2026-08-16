# Phase 2 Slice 1 brief — type-carrying checker (the typed-core spine)

Input for spec-writer. Phase 2 (typed core) is an epic; this is its first slice.
Read alongside [../DESIGN.md](../DESIGN.md), [../ROADMAP.md](../ROADMAP.md), and
[../CLAUDE.md](../CLAUDE.md). Everything here is scoped to Slice 1 only, and builds on
the Phase 0/1 compiler already on `main` (lexer/parser/checker/IR/QBE-emit/driver/REPL).

## Why this slice, and why it is worth doing alone

Phase 2 bundles the numeric tower, structs, enums/ADTs + match, optional/pointer, the
Copy marker, and fixed arrays. The load-bearing new machinery under all of it is one
thing: **each stack slot carries a `Type`, and the checker unifies types (not just
depth) through each body and at branch join points.** Slice 1 delivers exactly that
spine and nothing else. The remaining Phase 2 features hang off it in later slices.

The checker today is arity-only. It parses the type name in an effect comment
(`( int int -- int )`) into a string and then ignores it: the virtual stack it
simulates is a depth counter, and builtins are `(inputs, outputs)` arity pairs. With a
single implicit type there is no type mismatch to detect, so the spine cannot be proven.
Slice 1 introduces a **second** type, `bool`, precisely so join-unification and
operand-type checks become real: two types means a mismatch is now possible.

## Type set for Slice 1

Exactly two frontend types: **`i64`** and **`bool`**. No other numeric types, no
aggregates, no pointers. The numeric tower (`i32`/`u8`/`f64`/conversions) is Slice 2.

## Decisions locked for this slice

1. **`int` becomes `i64`, hard rename, no alias.** Slot types are concrete from here on.
   Update the examples (`gcd.sth`, `factorial.sth`, `lerp.sth`) and the Phase 0/1
   goldens that spell `int`. An `int` alias is future clutter and is not added.
2. **`true`/`false` are `bool` literals** (a new literal term kind, analogous to the
   existing integer literal). Otherwise `bool` would be producible (via comparisons) but
   not writable, making any program contrived.
3. **`bool` is a distinct IR type** (`IrType::Bool`), lowering to QBE `w` (0/1). It is
   not frontend-only sugar over `i64`: the type flows through the IR to the backend, in
   keeping with "types are real now" and the backend-neutral-IR invariant.
4. **`.` (print) stays `( i64 -- )`.** No bool printing in Slice 1. A program shows bool
   working end to end by *branching* on it and printing an `i64`.
5. **Stack shuffles stay structural and type-transparent, not polymorphic-signatured.**
   `dup`/`drop`/`swap`/`over`/`rot` are handled in the checker as structural stack
   rearrangements that move whatever concrete slot types are present (so `dup` of a
   `bool` works and yields two `bool`s), rather than being pinned to `i64` or given
   polymorphic signatures. Honest polymorphic *signatures* and monomorphisation are a
   Phase 4 formalisation (ROADMAP Phase 4); in Slice 1 the shuffles remain built-in
   structural ops, which is both less special-casing than i64-pinning and avoids making
   `bool` a second-class value. The backend already copies/reorders slot values for the
   shuffles; it must now carry the `bool` (`w`) width as well as `i64` (`l`).

## Typed builtin signatures

Replace the arity table with a typed-effect table:

- `+`, `-`, `*`, `mod` : `( i64 i64 -- i64 )`
- `=`, `<`, `>` : `( i64 i64 -- bool )` (the comparison result IR type becomes `Bool`;
  QBE comparisons already yield a `w` 0/1, so this is a type-label change at the op)
- `.` : `( i64 -- )`
- `dup`/`drop`/`swap`/`over`/`rot` : structural, per decision 5 (not table entries with
  fixed types)

## Type checking behaviour

- The checker simulates a **stack of concrete `Type`s**, not a depth counter. Every
  operand is checked against the expected type; a mismatch is a sharp, located error.
- **`if` requires a `bool`** on top for the condition (Phase 0 consumed an int). The
  `then`/`else` arms must agree at the join on **both depth and per-slot type**; a
  disagreement is an error that names the differing types.
- A word's body must produce exactly the **declared output types** (not just the right
  count); a mismatch against the effect comment is an error.
- Type names in effect comments are resolved against the known set; an **unknown type
  name** (anything other than `i64`/`bool`) is a reported error.
- The REPL path (`infer_line`, the carried virtual stack) carries **types**, not just
  the single 8-byte slot of Phase 1. A bare line is still inferred against the carried
  stack; the carried stack now records a `Type` per slot. (Phase 1's byte-buffer layout
  logic already anticipated richer slots; `bool` and `i64` are both 8 bytes on the
  buffer for Slice 1, so buffer marshalling need not change size, only carry the type.)

## Frontend `Type` vs backend `IrType`

Keep them distinct so the IR stays backend-neutral. Frontend `Type` = `{ I64, Bool }`
(it will grow in later slices). `IrType` gains `Bool` alongside the existing `Int`/`Ptr`.
Lowering maps `Type -> IrType`. Do not collapse `bool` into `Int` in the IR.

## Diagnostics (tested as behaviour, per CLAUDE.md)

Each of these must produce the *right* error, asserted on message content, not just a
failure:

- **`if` condition not bool:** e.g. `5 if 1 else 2 then` reports something like
  `expected` bool`, found` i64`` at the `if`.
- **Operand type mismatch:** e.g. `+` applied with a `bool` operand reports
  `expected` i64`, found` bool``.
- **Branch join type mismatch:** a `then` arm leaving `i64` and an `else` arm leaving
  `bool` reports that the arms disagree, naming the slot types.
- **Declared-output type mismatch:** a word declared `( i64 -- bool )` whose body leaves
  an `i64` reports the mismatch against the declared effect.
- **Unknown type name:** an effect comment naming an unknown type reports
  `unknown type` foo``.

(Exact wording is the implementer's; the tests assert the salient substrings and the
type names, following the existing diagnostic style.)

## Goal and exit criteria

Deliver the typed spine: a checker that carries a `Type` per stack slot and rejects type
errors sharply, with `i64` and `bool` as the two concrete types.

**Exit:**

1. `int` is renamed to `i64` throughout; `gcd`, `factorial`, `lerp` compile as `i64` and
   still produce `5` / `120` / `30` (Phase 0 goldens updated, still green).
2. `bool` is a real, distinct type: `true`/`false` literals parse; comparisons produce
   `bool`; `if` requires a `bool`.
3. Type mismatches are sharp compile errors (the five diagnostics above).
4. Branch join points unify on **type**, not just depth.
5. The Phase 1 REPL still works and its carried stack now tracks types; a bare line that
   would type-error reports it and the session survives (existing Phase 1 goldens green,
   updated for the `int`->`i64` spelling).
6. **Dogfood:** a small typed program that exercises `bool`, e.g.
   `: sign ( i64 -- i64 ) 0 > if 1 else 0 then ;`, compiled to a native binary (and
   runnable in the REPL), plus at least one negative golden proving a type error is a
   sharp diagnostic.

## Out of scope for Slice 1

The rest of the numeric tower (`i32`/`u8`/`f64`, conversion words, literal-to-target
adoption, `i128`/`u128`) which is Slice 2; structs/records; enums/ADTs and `match`;
fixed-size arrays; optional and non-null pointer types; the `Copy`/affine marker;
polymorphic shuffle signatures and monomorphisation (Phase 4); printing `bool`; any
heap or move semantics (Phase 3). All later slices or later phases.

## Current state / codebase anchors

- `src/check.rs`: `Arity = (usize, usize)` and `builtin_table()` (the arity table to be
  replaced by a typed-effect table); `check`, `check_def`, `check_word`, `infer_line`,
  and the shared depth simulation `check_terms`/`check_term`; `Ctx::Word`/`Ctx::Line`.
  Its module doc already says "Arity only for now; type unification is a later ROADMAP
  phase" — this slice is that phase.
- `src/ast.rs`: `StackEffect` with typed slots that currently carry a `ty: String`
  (the type name parsed but ignored); the term kinds (integer literal, call, `if`).
  A frontend `Type` enum and a `bool` literal term kind land here (lowest common
  ancestor of parser/checker), unless a growth signal argues otherwise.
- `src/ir.rs`: `IrType` (currently `Int`/`Ptr`); comparison and `if` lowering; the
  line-wrapper lowering (`lower_line`) that marshals typed slots for the REPL.
- `src/backend/qbe.rs`: emission of `l`/`w` typed values; comparison ops; `if` control
  flow; must carry `Bool` as `w`.
- `src/lexer.rs`, `src/parser.rs`: integer-literal lexing/parsing to extend with
  `true`/`false`.
- `examples/gcd.sth`, `examples/factorial.sth`, `examples/lerp.sth`: `int` -> `i64`.
- `tests/phase0.rs`, `tests/phase1.rs`: goldens updated for the rename; new typed
  positives and negatives added.

## Test plan

- **Goldens:** the three Phase 0 programs recompiled as `i64` (5/120/30 unchanged); a
  new `bool`-branching program (the `sign` dogfood) compiled to a binary and run; the
  five negative diagnostics above as negative goldens asserting the right error text; the
  Phase 1 REPL goldens updated for the rename, plus a REPL line that type-errors and the
  session survives.
- **Unit tests** beside each extended stage (per CLAUDE.md: happy path + at least one
  error/edge case), named `thing_condition_expected`:
  - checker: type propagation through a body; `if`-wants-bool; branch-join type
    unification (agree and disagree); operand type mismatch; unknown type name; declared
    output type mismatch; shuffle type-transparency (`dup`/`swap` on a `bool`).
  - parser/lexer: `true`/`false` literal parsing.
  - ir/backend: `bool` lowers to `w`; comparison result is `Bool`.
- Diagnostics are behaviour: every negative asserts the *right* message and the type
  names, not merely that it failed.
