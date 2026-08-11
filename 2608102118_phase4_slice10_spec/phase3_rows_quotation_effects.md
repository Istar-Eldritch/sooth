# Phase 3 — rows inside a quotation effect

Slice 10a, phase 3 of 7. Delivers **R4, R5, R6, R7, R8**, and **R10's row half**.

## What landed

- **`PolyType::Quotation` grows two trailing `Option<u32>` fields**
  (`src/ast.rs`) — the input/output row variable, if any, in the
  signature's own row id space (`PolySig::row_in`/`row_out`). Mirrors
  `PolySig` exactly, per R7. `RawTy::Quotation` (`src/parser.rs`) grows the
  same two fields on the parse side.
- **`PolyBuilder` grows a shared row-name id table** (`row_names`/
  `row_index`), interned incrementally as parsing proceeds rather than only
  at `finish()`. `set_row` (the existing top-level `..s` recorder) now
  interns into this table and stores the resulting id directly, so
  `PolySig::row_in`/`row_out` are `Option<u32>` from the moment they're set,
  not a string converted at the end. `finish()` simplifies to a plain field
  move — no more local closure reconstructing a second `row_names` table.
- **`PolyBuilder::quotation_row_id`** (R4): a `..`-prefixed name mentioned
  inside a quotation effect must already be interned — i.e. it must be the
  signature's own top-level row (or another row already mentioned earlier
  in the same signature). A lookup miss covers both failure shapes the
  requirement names: a fresh name, and a signature that declared no
  top-level row at all (the table is empty). Located at the mention's own
  span.
- **`parse_poly_quot_list`** (the inside of a quotation effect's input/
  output list) now checks its leading token for a `..`-prefixed row mention
  before falling into the ordinary slot loop, exactly as `parse_poly_slots`
  already does for a signature's own top-level row. Returns the row id
  (if any) and its `(name, span)` alongside the parsed slots, so the caller
  can render R5's located one-sided-row error.
- **`parse_poly_quotation_inner`** enforces R5 once both sides are parsed:
  both a row or neither (a one-sided row is a located error naming the row
  and its span); when both sides declared a row, the two ids must be equal
  for 10a (a differing pair is the spec's fixed exact-text error — no span,
  since the text is pinned verbatim and doesn't need one). On success it
  builds `RawTy::Quotation(inputs, outputs, is_inline, row_in, row_out)`.
- **R6 — the concrete fold is suppressed whenever a row is set**, independent
  of `~`: `raw_to_poly_type`'s Quotation arm only folds to
  `Concrete(Type::Quotation`/`InlineQuotation)` when every slot is concrete
  *and* neither row field is `Some`. `QuotEffect` has nowhere to put a row
  (R7), so folding a row-bearing effect would silently destroy it before any
  splice ever sees it — exactly the defect the spec calls out for
  `~[ ..s i64 -- ..s ]`, which is fully concrete slot-by-slot and would
  otherwise fold.
- **R8 falls out of the representation, not new logic**: the row was never
  a slot in `ins`/`outs` (it's the separate field added above), so
  `unify_poly_input`'s pointwise arity check (`ins.len() != eff.inputs.len()`)
  already excludes it, before and after this phase.
- **Renderers made row-aware (R10's row half)**:
  - `poly_type_str`'s Quotation arm renders the row as the leading element
    of its side (`sig.row_var_names[id]`), exactly mirroring `poly_sig_str`'s
    existing `render_row` closure for the top-level row.
  - `unify_poly_input`'s two quotation-mismatch branches (the `is_quotation_type`
    let-else, and the arity-mismatch branch) no longer fabricate a `Type` via
    the old `poly_quotation_concrete_hint` (which built `[ -- ]` for the
    let-else — "a type nobody wrote" per the spec — and could never have
    shown a row even in the arity branch, since `Type::Quotation`'s
    `QuotEffect` has no row field to hold one). Both now render the
    *declared* `PolyType` itself through `poly_type_str(pty, sig)`, via a new
    `poly_quotation_type_mismatch_error` (a `type_mismatch_error` twin taking
    the expected side as an already-rendered `&str`). This fixes the stale
    `[ -- ]` fallback and makes the row visible in both messages, in one
    change, since both problems had the same cause (trying to express a row
    inside a `Type`).
- Every other pre-existing `PolyType::Quotation` match arm across
  `ast.rs`/`check.rs`/`ir.rs`/`repl.rs`/tests — `collect_poly_concrete`,
  `poly_copy_gate`, `poly_op_on_variable_error`, `reject_poly_quotation_anywhere`,
  `audit_poly_input_quotation`, `poly_input_is_quotation`, `apply_subst`,
  `subst_polytype`, `remap_poly_type`, and the two test fixtures — extended
  to the 5-field pattern via `_, _` or `..`, with no behavioural change: none
  of them need to know about a row this phase (grounding it against a live
  stack is phase 4's job, R9).

## No grounding yet

`apply_subst` is explicitly left untouched (a note added explaining why:
splicing a caller region into its *interned* `&'static QuotEffect` would mint
an effect no literal or forwarded parameter could ever equal again — R9's
own reasoning, arriving early because the match arm needed edited anyway).
Nothing in this phase reads a row against a concrete call-site stack.

## Tests

Five new unit tests in `src/parser.rs`, beside the existing row tests
(`parse_row_variable_records_both_sides` etc.):

- `parse_row_in_quotation_effect_survives_the_concrete_fold` (R6/R7): asserts
  `~[ ..s i64 -- ..s ]` inside `my-times`'s signature shape stays
  `PolyType::Quotation` with `ins.len() == 1` (the row is a field, not a
  slot) and both row ids populated and equal.
- `parse_row_in_quotation_effect_fresh_name_is_error` / `..._no_top_level_row_is_error`
  (R4): a quotation-effect row naming something other than the signature's
  own top-level row is rejected, both when a different top-level row exists
  and when none does.
- `parse_row_in_quotation_effect_one_sided_is_error` (R5): a row on one side
  of a quotation effect only.
- `parse_row_in_quotation_effect_differing_output_row_is_error` (R5): two
  *distinct* top-level rows (`..s` in, `..t` out — each already known by the
  time the nested quotation mentions it, since each is the leading token of
  its own top-level side) referenced on the quotation's two sides is the
  fixed exact-text error:

  ```
  error: a loop body cannot change the shape of the carried region: `..s` in, `..t` out
  note: 10c lifts this for a word without a back-edge
  ```

## R20 — mutation evidence

Each guard reverted individually against a scratch copy of `src/parser.rs`,
confirmed to flip its test, then restored (verified byte-identical via
`diff` afterward):

| Guard | Reverted to | Flips |
| --- | --- | --- |
| R6 fold suppression (`if !has_row`) | drop the guard, always fold when concrete | `parse_row_in_quotation_effect_survives_the_concrete_fold` |
| R4 `quotation_row_id` | always intern/accept any name | both `..._fresh_name_is_error` and `..._no_top_level_row_is_error` |
| R5 one-sided check | drop both `(Some, None)`/`(None, Some)` arms | `parse_row_in_quotation_effect_one_sided_is_error` |
| R5 differing-row check | drop the `a != b` arm | `parse_row_in_quotation_effect_differing_output_row_is_error` |

## Green

`cargo fmt --check && cargo clippy -- -D warnings && cargo test`: 778 lib
tests (5 new), full existing integration suite (including `qbe_baseline`)
unchanged. `lib/combinators.sth` byte-unchanged; `git diff --stat` touches
only `src/{ast,check,ir,parser,repl}.rs`, no new test files (the new
coverage lives beside the existing row unit tests in `src/parser.rs`, since
nothing here is user-visible behaviour needing an integration golden yet —
that arrives with grounding in phase 4).
