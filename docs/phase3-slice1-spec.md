# Phase 3 Slice 1 — Linear analysis + move + `dup`-gate + explicit `drop` (spec)

## Context

The first cut of the **linear spine**, isolated from heap. This slice makes the compiler
enforce linear (use-*exactly*-once) discipline: values move by default, a second use of a
moved value is a compile error, `dup`/`over` are gated on `Copy`, `drop` runs a destructor,
and **forgetting to dispose a linear value is a compile error** (no auto-drop, no join
reconciliation; Copy and linear are handled symmetrically for disposal). It is proven on a
**test-only builtin drop-spy** whose destructor prints, so drop count/order/timing are
golden-observable. Aggregates are in scope via **destructure-whole** (no partial moves). No
heap, no references, no RC, no fds. Design is locked in `docs/phase3-slice1-brief.md` (D1-D8)
and the target semantics are in `DESIGN.md` "## The linear spine".

This slice **changes the compiler** (`src/**`), unlike the Slice 7 dogfood.

## Requirements (traceable)

- **R1 (D1)** — Linear, exactly-once. No auto-drop anywhere. A linear value must be consumed
  (moved out as a declared output, moved into a consuming word, or `drop`ped) exactly once.
- **R2 (D1)** — Copy/linear symmetry for disposal: a surplus value at scope end is an error
  for both. The only difference is that `drop` runs a destructor on a linear value and a
  linear value *must* be consumed, while a Copy value may be discarded freely.
- **R3 (D2)** — Mentioning a linear local moves its value out; a second mention is a
  `use after move` error that names the move site. No mid-body rebinding (existing rule).
- **R4 (D3)** — `dup` and `over` on a non-`Copy` value are compile errors with a diagnostic
  of the DESIGN.md form ("cannot `dup` a value of type X; X is linear …"). `swap`/`rot`/`nip`
  (pure reorderings) are legal on linear values.
- **R5 (D4)** — `drop` is the universal disposal primitive: it runs the destructor on a
  linear value, discards a Copy value. (User-overridable `drop` is Phase 4; here destructors
  are compiler-known.)
- **R6 (D5)** — A **test-only builtin linear primitive** (the drop-spy) exists: an internal
  name (not user-facing surface, not in the tutorial), carrying an `i64` tag, with a
  compiler-known destructor that prints a deterministic line (`drop <tag>`). It is the sole
  linear primitive this slice introduces.
- **R7 (D6)** — A struct/enum is **linear iff any field/variant-payload is linear**
  (transitive). Plain-data aggregates stay `Copy`, unchanged.
- **R8 (D6)** — `dup`/`over` on a linear aggregate is an error. `S` (construct) moves fields
  in. `S>` (destructure) consumes the aggregate and pushes all fields (each linear field then
  tracked per R3).
- **R9 (D6)** — `S>fi` (get) keeps its `( S -- field )` consuming effect for **any** field;
  on a linear receiver it additionally runs drop glue on the non-extracted fields. Existing
  Copy code is unchanged (drop-the-rest is a no-op when the rest is Copy).
- **R10 (D6)** — `S|>fi` is a **new** non-consuming `( S -- S field )` peek, **Copy fields
  only**; on a linear field it is a compile error (workaround: `S>`). It transfers no
  ownership, needs no reference machinery.
- **R11 (D6)** — `S<fi` (set) is allowed on linear aggregates and **drops the overwritten
  field if it is linear**; the other linear fields transfer via the existing blit-move (old
  shell consumed, never dropped).
- **R12 (D6)** — The compiler synthesizes **recursive/tag-dispatched drop glue**: struct →
  drop its linear fields in declaration order; enum → dispatch on the tag, drop the active
  variant's linear payload.
- **R13 (D7)** — Disposal is explicit; nothing is auto-inserted. A linear value left on the
  stack beyond the declared outputs is a stack-effect error (reusing `check_outputs`); a
  linear local never consumed by scope end is an error (`linear value X is never consumed;
  drop it or return it`).
- **R14 (D7)** — Branch joins: no reconciliation. Stack shapes already unify across
  `if`/clause arms, so only a local's move-state can diverge. A local moved in one arm and
  live in another surfaces as use-after-move (if used after the join) or unconsumed-linear
  (if not). The compiler errors; it does not insert compensating drops.
- **R15 (D8)** — Deferred, and enforced as located errors where reachable: a linear value
  live across a Slice 6 back-edge is a `linear values across a loop are not supported yet`
  error (Copy loops unaffected). Out of scope entirely: partial/field-level moves (excluded
  by destructure-whole), user-overridable `drop` (Phase 4), heap/refs/RC/fds.

## Delivery phases (dependency-ordered)

### Phase 1 — Isolated linear core on bare drop-spy values `[hard]`

The load-bearing novel analysis, with **no aggregates** yet, so a bug is in the analysis,
not in struct/enum glue.

- **Copy-vs-linear predicate** over `Type`: every current type is `Copy`; the new drop-spy is
  linear. A single query `is_copy(&Type) -> bool` (or `Linearity`) threaded where needed.
- **The drop-spy builtin**: an internal-named primitive type + a constructor word
  (`( i64 -- Spy )`) in the builtin table; its compiler-known destructor emits a runtime
  print of the tag. Fence it by name/docs as a test intrinsic (not tutorial surface).
- **Move-tracking on linear locals** (forward flow through straight-line, `if`/`else`, and
  clause bodies): first mention moves; a second mention is `use after move` naming the move
  site.
- **`dup`/`over` gate**: reject on non-`Copy`, DESIGN.md-form diagnostic.
- **`drop` lowering**: on a linear value, emit the destructor call (spy prints its tag); on
  Copy, discard as today.
- **Linear disposal check** (extend `check_outputs` @check.rs:554 + a locals-linearity pass):
  a surplus linear value on the stack, or a linear local never consumed, is a compile error.
  Copy surplus stays the existing error; do not regress it.
- **Deferred guard (R15)**: a linear value live across the self-tail-call back-edge is a
  located "not supported yet" error.
- **Changes**: `src/check.rs`, `src/ir.rs`, `src/backend/qbe.rs`, `src/lexer.rs`,
  `src/parser.rs`, `src/ast.rs` (drop-spy constructor surface), `tests/phase0.rs`.
- **Exit**: criteria 1, 2, 3, 4, 10 pass as native goldens; `cargo fmt --check && cargo clippy
  -- -D warnings && cargo test` green; all pre-existing examples/tests still pass.

### Phase 2 — Struct aggregates via destructure-whole

- **Linear propagation** for structs: linear iff any field linear (transitive).
- **`S>fi` drop-the-rest** on a linear receiver (R9); **`S<fi` drop-on-overwrite** (R11);
  **recursive struct drop glue** (R12, struct case).
- **Changes**: `src/check.rs`, `src/ir.rs` (`lower_struct_word` @ir.rs:1592),
  `src/backend/qbe.rs`, `tests/phase0.rs`.
- **Exit**: criteria 5, 6, 8 pass as native goldens; green; no regression.

### Phase 3 — The `S|>fi` non-consuming peek word

- New surface form `Struct|>field`: lexer/parser/ast, checker typing (`( S -- S field )`,
  Copy field only, error on a linear field), inline lowering (non-consuming projection).
- **Changes**: `src/lexer.rs`, `src/parser.rs`, `src/ast.rs`, `src/check.rs`, `src/ir.rs`,
  `tests/phase0.rs`.
- **Exit**: criterion 7 passes (peek leaves the aggregate live; peek on a linear field is a
  compile error); green; no regression.

### Phase 4 — Enums: tag-dispatched drop glue

- **Synthesized tag-dispatched destructor** for dropping a linear enum that is never matched
  (R12, enum case); a matched clause consumes/drops its exposed payload per R3/R13.
- **Changes**: `src/check.rs`, `src/ir.rs`, `src/backend/qbe.rs`, `tests/phase0.rs`.
- **Exit**: criterion 9 passes; green; no regression.

### Phase 5 — Regression sweep, example, REPL parity, doc consistency

- Confirm every existing example and test stays green.
- Add a small `examples/*.sth` demonstrating linear construction, threading, and explicit
  `drop` (using the drop-spy or a struct containing it, if the spy is reachable from surface;
  otherwise a doc-example in a golden).
- REPL parity golden (`tests/phase1.rs`) for the core behaviours (drop runs once; forgetting
  is an error is checked at compile, so a REPL golden asserts the successful disposal path).
- Confirm `DESIGN.md`/`ROADMAP.md` linear framing matches what shipped.
- **Changes**: `tests/phase0.rs`, `tests/phase1.rs`, `examples/*.sth`.
- **Exit**: criterion 11 (no regression) holds; green.

## Criterion → test map (all runnable native/REPL goldens, not IL-string asserts)

| # | Criterion | Test (phase) |
|---|---|---|
| 1 | `dup` on a spy (or struct-of-spy) is a compile error with the linear diagnostic | `dup_of_linear_value_is_error` (P1) — asserts message + type name |
| 2 | Use-after-move: a moved spy local mentioned twice errors, naming the move site | `use_after_move_of_linear_local_is_error` (P1) |
| 3 | A destructor runs exactly once at explicit `drop` (tag printed once) | `explicit_drop_runs_destructor_once` (P1, native stdout) |
| 4 | Forgetting is an error: surplus spy on stack / unconsumed linear local | `unconsumed_linear_value_is_error` + `surplus_linear_on_stack_is_error` (P1) |
| 5 | Destructure-whole: `S>` a struct-of-spies, drop fields; each runs; order asserted | `destructure_whole_drops_each_field_in_order` (P2, native stdout) |
| 6 | `S>fi` on a linear struct extracts one field and drops the rest | `get_field_drops_the_rest_on_linear_struct` (P2, native stdout) |
| 7 | `S|>fi` peeks a Copy field leaving the struct live; on a linear field it errors | `peek_copy_field_keeps_struct` + `peek_linear_field_is_error` (P3) |
| 8 | `S<fi` on a linear struct drops the overwritten spy, keeps the rest | `set_field_drops_overwritten_linear_field` (P2, native stdout) |
| 9 | Unmatched linear enum drops the active variant's spy via tag-dispatch; matched clause disposes payload | `drop_of_linear_enum_dispatches_on_tag` + `clause_body_disposes_linear_payload` (P4) |
| 10 | Branch: spy consumed in both arms is ok; one-arm divergence is an error | `both_arms_consume_linear_ok` + `divergent_arm_move_is_error` (P1) |
| 11 | No regression: gcd/factorial/vm/stack/shapes/vectors/rgb/countdown still build+pass | existing goldens + REPL parity (P5) |

## Phases JSON

```json
{"phases":[
  {"phase":1,"focus":"Isolated linear core on bare drop-spy values: Copy-vs-linear predicate, the test-only drop-spy builtin primitive + constructor with a print-on-drop destructor, move-tracking on linear locals (use-after-move error naming the move site), dup/over gated on Copy, drop runs the destructor, and the disposal check (surplus linear stack value and unconsumed linear local are compile errors via extended check_outputs). No aggregates. A linear value across the Slice 6 back-edge is a located not-supported-yet error.","difficulty":"hard","changes":["src/check.rs","src/ir.rs","src/backend/qbe.rs","src/lexer.rs","src/parser.rs","src/ast.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criteria 1,2,3,4,10 pass as native goldens; fmt/clippy/test green; existing examples and tests unregressed"},
  {"phase":2,"focus":"Struct aggregates via destructure-whole (no partial moves): linear-iff-any-field-linear propagation (transitive); S>fi stays ( S -- field ) consuming and drops the non-extracted fields on a linear receiver; S<fi functional update drops the overwritten linear field; synthesized recursive struct drop glue.","changes":["src/check.rs","src/ir.rs","src/backend/qbe.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criteria 5,6,8 pass as native goldens; green; no regression"},
  {"phase":3,"focus":"The new S|>fi non-consuming peek word: surface form Struct|>field through lexer/parser/ast, checker typing as ( S -- S field ) for Copy fields only with a compile error on a linear field, and inline non-consuming projection lowering.","changes":["src/lexer.rs","src/parser.rs","src/ast.rs","src/check.rs","src/ir.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criterion 7 passes; green; no regression"},
  {"phase":4,"focus":"Enums: synthesized tag-dispatched drop glue for dropping a linear enum that is never matched, and clause-body disposal of the exposed linear payload.","changes":["src/check.rs","src/ir.rs","src/backend/qbe.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criterion 9 passes; green; no regression"},
  {"phase":5,"focus":"Regression sweep (all existing examples and tests stay green), a small new examples/*.sth demonstrating linear construction/threading/explicit drop, a REPL parity golden for the disposal path, and DESIGN.md/ROADMAP consistency confirmation.","changes":["tests/phase0.rs","tests/phase1.rs","examples/linear_demo.sth"],"tests":["tests/phase0.rs","tests/phase1.rs"],"exit":"criterion 11 (no regression) holds; green"}
]}
```
