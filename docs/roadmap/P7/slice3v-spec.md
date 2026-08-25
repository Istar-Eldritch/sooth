# Phase 7 Slice 3v: dropping and storing a linear-capturing quotation

**Status:** Implemented (4 phases, landed on `impl/slice3v_spec-2608251924`)
**Discovery:** `docs/roadmap/P7/slice3v-brief.md`
**Predecessor:** `docs/roadmap/P7/slice3h-spec.md`, which shipped `owning [ … ]` and the two
restrictions this slice lifts for three positions.

## Problem

S3h's owning closure could only be discharged by `call`, because nothing could invoke a
per-value disposer. Two restrictions followed:

1. `drop` on an owning closure was a located rejection, twinned on the concrete
   (`check.rs`) and generic (`check/poly.rs`) `"drop"` arms, both calling
   `cannot_drop_owning_quotation_error`.
2. An owning closure could not sit in any aggregate position:
   `audit_quotation_type_registries` carved out only a *plain* `Type::Quotation` struct field
   (D4) and array element, so struct field, variant field and cell payload were all rejected.
   The glued spelling `^owning [ -- ]` never even reached the audit: `split_owning_cell_word`
   resolved the remainder `"owning"` as an unknown type name.

Lowering could not have absorbed a lifted gate either: `emit_drop` had no quotation arm,
and `field_is_linear`/`layout_field_is_linear` answered `false` for one, so no container's
`is_linear` fold saw an owning field and no destructor was synthesized. Deleting the checker
gates alone would have leaked the capture and the heap env block on every container `drop`.

## Design rulings (as shipped)

### R1 — The disposer is a third word in the shared quotation value, keyed on the construction site

`Type::OwningQuotation` carries only the declared effect, so two closures with identical
effects and different capture sets are one type: nothing type-directed can discriminate them.
The value gained a third slot instead, written at `materialize_quot_value`
(`src/ir/func_builder/quotation.rs`), the one place the captures' concrete types are known.
`quotation_layout` (`src/ir/types.rs`) is `{ code, env, disposer }`, `3 * word_width`, with
`disposer_offset` on `QuotLayout`; every lowering site reads offsets from that struct.

**Both flavours carry the third word.** `:Q{n}` is keyed on effect alone, collapsing a plain
and an owning quotation of one effect to a single symbol; a per-variant width would re-key that
symbol on effect *and* owning-ness across every site that maps the two together, to save 8
bytes on materialized closures. A plain quotation's disposer slot is always null, mirroring the
existing null-env convention. **Canary: an edit to `src/ir/types.rs`'s variant-identity doc
comment claiming the two widths differ means this ruling was reversed by accident.**
`src/backend/qbe.rs`'s hardcoded `type :Q{idx} = { l, l, l }` is the only literal spelling of
the width.

### R2 — The disposer's body composes existing per-type disposal

`synthesize_owning_disposer` (`src/ir/func_builder/mod.rs`, symbol
`quot_disposer_symbol` = `{symbol}__dispose`) takes the env block as its sole parameter, and for
each capture at its `owning_env_slots` offset applies the same `field_is_linear` gate plus
`slot_value` + `emit_drop` fold a struct's field glue (`drop_level_fields`) performs, then frees
the block. A borrowed capture is `IrType::Ptr`, never linear, and is skipped for free, but its
slot still counts so later offsets do not shift. A capture-free literal allocates no block,
mints no function, and stores a null disposer.

`drop` and `call` are alternatives, never both: `call` runs the body, whose prologue
(`bind_owning_env`) frees the same block and whose own logic consumes the same captures.

### R3 — `emit_drop` gains one `OwningQuotation` arm

It loads the disposer slot, compares against null, and indirect-calls it with the loaded env
slot in a guarded block. This is the only `emit_drop` arm that is not a single instruction in
the caller's block: it seals the current block and opens two. The nullness is necessarily a
*runtime* branch, since `emit_drop` sees the value's `IrType` and never its slot contents.
Both consumers reduce to this arm: a bare `drop`, and a container's synthesized destructor
reaching the field through the unmodified struct/enum field glue once R5 makes it linear.

### R4 — The twinned `drop` rejection is deleted

Both arms and `cannot_drop_owning_quotation_error` are gone. The generic twin stays reachable
even though a generic word cannot *declare* an owning parameter: it can call a word returning
one, so an owning closure arrives through the body rather than the signature.

### R5 — Two linearity folds widened, and only them

`field_is_linear` and `layout_field_is_linear` (`src/ir/layout.rs`) answer `true` for
`IrType::OwningQuotation`, alongside `OwnedCell`. Plain `IrType::Quotation` stays on the
`_ => false` wildcard: no new `Copy` obligation, no IL churn for a program without `owning`.
`crate::check::is_copy` needed no change; its `Type::OwningQuotation` arm has answered `false`
since S3h, so the gap was purely IR-side.

### R6 — Containment lifted for three positions only

`audit_quotation_type_registries` gained three separate carve-outs, not two mirrored ones:
the struct-field one widened to `Type::Quotation(_) | Type::OwningQuotation(_)`, and two new
owning-only ones on the enum-variant loop and the owned-cell loop (neither of which ever had a
plain-quotation exception). The array loop keeps its plain-only carve-out and the
reference-referent loop gains none, so an array element, a reference referent and an `extern:`
boundary stay exactly as rejected as before. `type_node` (`src/check/declarations.rs`) treats an
owning field as a containment leaf, load-bearing rather than vacuous now: the value is a fixed
three-slot aggregate whose captures live behind the env pointer, so `type: Box q owning [ Box -- ] ;`
is finite. `check/terms.rs`'s `!`/`+!` materialization boundary needs no owning arm: the flavour
is `is_linear`, so `check_access_word` rejects the overwriting store outright.

### R7 — The glued `^owning` parser gap

`split_owning_cell_word` gained a `remainder == OWNING_QUOTATION_KEYWORD` arm that reads the
effect rows through the shared `quotation_effect_opens_here`/`parse_quotation_effect_rows`
pair (raising the existing `owning_without_effect_error` when they are absent) and folds the
result through `intern_owned_cell_type` like every other arm. No lexer change; the spaced form
already worked through `parse_type_expr`'s `owning_quotation_ahead()` dispatch, and the two
spellings remain genuinely distinct code paths.

### Finding (phase 3) — the phase 2 decomposition missed a lowering arm

`load_owned_payload`/`store_owned_payload` (`src/ir/func_builder/word_families.rs`) had no
quotation case, so an owning-quotation cell payload newly admitted by R6 would have hit the
scalar `FieldLoad`/`FieldStore` fallback instead of the aggregate blit its layout needs. Fixed
in phase 3 rather than deferred, since the cell carve-out's own goldens fail without it. Both
arms admit `IrType::OwningQuotation` only: a plain quotation payload stays on the scalar
fallback and rejected by the checker, pre-staging no future D4 widening.

### R8 — The REPL override-epoch obligation is unreachable, not untested

R8's no-new-plumbing claim stands: the disposer calls `emit_drop`, whose arms already resolve
`struct_drop_symbol`/`enum_drop_symbol` against the `drop_generation` that
`apply_drop_generations` sets before lowering. What was false is this spec's own earlier claim
that the field/`drop` shape sidesteps the REPL's materialization limit. A disposer exists only
for a *materialized* closure, and storing one in a field is exactly what forces materialization,
so the session dies in `ld` on a non-PIC relocation against `__quot0`/`__quot0__dispose` before
any epoch matters. Measured identical for a plain quotation with no `owning`, no disposer and no
third-word write, so it is P7.S3h's standing hazard, not a regression here. Every other REPL
route to a disposer is closed: an `owning` parameter is rejected on a spliced word, a real-call
quotation parameter is refused at the session boundary, and an inline literal `call` runs the
body rather than the disposer.

Delivered instead: a blocked-state tripwire (see Tests), not a skip. The closure must be built
and stored **on one line**; routing it through a session-defined factory word tests nothing,
since that definition line materializes on its own account and dies before `Box`'s admission,
R5 or R6 are reached.

## Out of scope

- **Array and slice element positions**, for an owning closure or any linear type: P7.S5. R6's
  carve-outs are additive over exactly three positions and must not grow a fourth here, even
  once R5 makes it look free.
- **P7.S3u** (trait objects / erased owners), parked.
- Polymorphism over plain-versus-owning quotation types, and an owning parameter on a spliced or
  generic word: unchanged since S3h (`reject_owning_quotation_declarations`).
- Inline and static env storage for an owning closure: unchanged since S3h.
- A user `drop` overload on a struct holding an owning field: `DropOverrides` is generic over
  any linear field and needed no new case.
- The REPL's inability to link a materialized quotation (R8). An owning closure joins that
  existing failure class.

**Known-stale comments, deliberately left** for whichever phase next opens each file, all
comment-only and functionally harmless: `src/backend/qbe.rs`'s `member_ty` arm calling an
owning-quotation field "unreachable"; the twinned `past_owning_frame_error(..., false)`
justification in `src/check/terms.rs` and `src/check/word_entry.rs`; and `src/parser.rs`'s
"the containment rule that rejects it" note. Each conclusion still holds post-R6 for unrelated
reasons.

**Two follow-ups, not delivered:**

- `past_owning_frame_error`'s `owning`-field hint (`src/check/captures.rs`) is never suggested by
  `quotation_captures_local_error` (`src/check.rs`), even though switching a plain quotation
  field to `owning` now fixes the by-value-linear-capture error R6 left reachable. A
  diagnostic-hint addition for whichever phase next touches that D3 site.
- R8's obligation is still assertable in-process: `src/repl.rs`'s `#[cfg(test)]`
  `destructor_symbols` helper builds real session registries through `apply_drop_generations`,
  so the same shape could lower an owning literal and assert the synthesized `__dispose` body
  names the epoch-suffixed `sooth_struct_drop_N`, with no `dlopen` and so no PIC problem. It
  proves less than the golden but is the difference between zero coverage and a unit guard.

## Invariants

- Every quotation value is three words wide, both flavours, sharing one `:Q{n}` symbol per
  effect (R1's canary).
- A null disposer means "nothing to dispose", exactly as a null env means "nothing captured";
  the check is at runtime, in the guarded block `emit_drop` opens.
- The disposer re-derives no per-type disposal: it is `field_is_linear` + `emit_drop`, the same
  primitives a struct's field glue uses, over an anonymous capture list.
- Destructor synthesis itself is unchanged. R5 is the whole of what makes the existing machinery
  see an owning field.
- R6's carve-outs are per-position and owning-only (except the pre-existing plain struct-field
  and array-element ones); a plain quotation gains no new position.

## Tests

End-to-end, `tests/phase7_slice3v.rs`, through the real binary, every negative pinning the exact
diagnostic: `dropping_an_owning_closure_disposes_its_capture_once` and its
`…_in_a_generic_body_…` twin (the R4 mutation guards, the second through a real generic body);
`call_and_drop_run_different_code` (R3's headline, two branches on one source);
`dropping_a_capture_free_owning_closure_skips_the_null_disposer`;
`an_owning_quotation_field_is_disposed_on_container_drop` and its variant-field sibling;
`an_owning_quotation_cell_payload_is_admitted_{spaced,glued}` (R6/R7, two code paths);
`an_owning_cell_of_owning_quotation_is_disposed_on_cell_drop`;
`an_owning_field_disposes_alongside_its_siblings_exactly_once` (order and exactly-once, against
a double-visiting field glue); and the blast-radius trio
`an_array_element_owning_closure_is_still_rejected`,
`a_reference_referent_owning_closure_is_still_rejected`,
`an_owning_cell_payload_of_a_plain_quotation_is_still_rejected`.

R8 ships as `explicit_repl_override_epoch_disposal_is_blocked_by_the_repl_link_limit`, asserting
the blocked state through the scripted REPL harness: `Box` is admitted, `__quot0__dispose` is
minted, `drop 7` never prints, the failure is `"cc" failed` rather than a diagnostic, and the
session survives. Reverting R6's struct-field carve-out refuses `Box` and fails it, so it
discriminates this slice rather than restating S3h. `a_plain_quotation_value_hits_the_same_repl_link_limit`
is the not-owning's-fault control. When the session-module PIC problem is fixed this fails, and
the fixer promotes it by asserting the disposal line.

Four S3h tests migrated out of `tests/phase7_slice3h.rs` (the two `…_is_rejected` field tests and
the two `dropping_an_owning_closure…_is_a_located_rejection` tests), and
`a_plain_quotation_keeps_its_two_word_layout_and_gains_no_allocation` became
`a_plain_quotation_still_carries_a_null_disposer_slot`.

Unit: `src/ir/types.rs` on the three offsets/size/align; `src/ir/func_builder/quotation.rs` on
`emit_drop` over a constructed `IrType::OwningQuotation` (disposer `FieldLoad`, null `Cmp`, and
the env load plus `CallIndirect` in the guarded block only, none in the entry block), with the
null half asserted value-side instead, since `emit_drop` cannot see slot contents;
`src/ir/layout.rs` on both widened folds; `src/parser.rs` on `^owning [ -- ]` and a `^Spy`
control; `src/check/audits.rs` on `audit_quotation_type_registries` directly, where R9's split of
the old five-position test lives as `owning_quotation_is_admitted_in_three_positions` plus
`owning_quotation_is_rejected_in_every_remaining_aggregate_position`.

Mutation-tested per phase: R3's null check (the call moves to the entry block, no `Cmp`/`Jnz`
pair remains), R4's two guards independently, R5's two folds independently, R6's three carve-outs
independently with the still-rejected goldens passing throughout, and R7's glued-form arm with
the spaced-form golden still passing.

## Phases (delivered)

1. Widen the quotation value to three slots, write a null disposer for both flavours at
   `materialize_quot_value`, sweep the `{ l, l }` QBE IL goldens and the stale two-slot comments
   (the comment sweep ran to two review rounds and reached `func_builder/`, `layout.rs`,
   `repl.rs`, `check/captures.rs`, `check/poly.rs`, `backend/qbe.rs`).
2. `synthesize_owning_disposer` plus R3's `emit_drop` arm, reachable at this point only from the
   unit tests, since no admitted program yet reaches it.
3. R4, R5, R6, R7, the four migrated tests, the R9 audit-test split, every new end-to-end golden
   bar the REPL one, and the out-of-scope-but-required owned-payload lowering arms (Finding).
4. R8's blocked-state tripwire and its control, plus the roadmap and S3h close-out. No source
   edit was needed, which is what the phase's out-of-bounds line demanded.
