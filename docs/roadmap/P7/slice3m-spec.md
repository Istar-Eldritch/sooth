# Spec: P7.S3m a declared quotation effect with two or more outputs interns an output bundle

**Status:** Implemented
**Discovery:** `docs/roadmap/P7/slice3m-brief.md`

## Problem

Multi-output return bundles were minted in one place, `intern_output_bundles`
(`src/check.rs:1020`, called from `:972`), which walked `module.words` and interned a bundle
for each declared word whose own output list had length >= 2. A *quotation* value's declared
effect (`[ i64 -- i64 i64 ]`) was never inspected. At lowering, `lower_indirect_call`
(`src/ir/func_builder/quotation.rs`) asks `bundle_of` for the tuple; with nothing interned it
got `None`, the `CallIndirect` produced no value, and the `unpack_bundle` that pushes the
outputs was skipped. Every output past the first was lost and the first consumer underflowed.

Confirmed repro: `: call_it ( [ i64 -- i64 i64 ] -- ) 3 swap call add . ;` panicked at
`calls.rs` (`bin: rhs`), identically for a concrete or a polymorphic `call_it`.

## Mechanism

`intern_bundle_struct`, `bundle_of`, and `word_ret_ty` are already shape-agnostic; the whole
gap was *discovery*. The fix is a strict widening of `intern_output_bundles`, with no
downstream change. Bundles are keyed by their exact type list, so a quotation tuple that
coincides with a word's re-interns to the same `StructId`.

**R1 -- discovery sites.** Beyond the existing word-output tuples, the walk now covers:

1. **Word inputs and outputs** (`w.effect.inputs`/`.outputs`): descend into each
   `TypedSlot.ty` looking for a nested `Type::Quotation`.
2. **Struct fields**, swept straight off `module.structs`.
3. **Array elements**, swept straight off `module.arrays`.
4. **Poly signatures** (`w.poly.inputs`/`.outputs`): `w.effect` is empty for a poly word, so
   without this site every poly quotation parameter is missed.

Sites 2 and 3 are registry sweeps, not signature recursion: every `Type::Struct`/`Type::Array`
naming one is an index into these registries, so sweeping them reaches a quotation nested at
any depth with no containment walk.

**R2 -- `Type` recursion (`collect_quotation_bundles`).** On a `Type::Quotation(eff)` with
`eff.outputs.len() >= 2`, record the tuple; then recurse through `eff.inputs`/`.outputs` (a
quotation whose own parameter is a multi-output quotation). Nothing else is descended:
reference referents, owned-cell payloads and enum variant fields are rejected outright by
`audit_quotation_type_registries` (`src/check/audits.rs`), and structs/arrays are the sweeps
above. `Type::InlineQuotation` (a `~[ ... ]` parameter) is deliberately excluded: a `~` is
always spliced at its call site and never reaches `lower_indirect_call`. This is the one place
in `check.rs` that does *not* use `is_quotation_type`.

**R3 -- poly signatures.** Only a top-level `PolyType::Concrete(Type::Quotation(..))` is handed
to R2. No wrapper recursion is needed: `audit_poly_input_quotation` rejects a quotation in a
poly array element, referent, cell payload, generic argument and quotation-effect row, and a
fully concrete composite (`[ [ i64 -- i64 i64 ] 2 ]`) folds to `Concrete` at parse time. A
variable-bearing `PolyType::Quotation` has no ground output tuple to key a bundle by and is
skipped.

Ordering is unchanged: discovery still runs at `check.rs:972`, after every type-level check and
`struct_generated_sigs`, before the per-instantiation `inst.out_arity >= 2` loop and the S3k
transitive fixpoint. A quotation parameter's effect is a fixed concrete shape read off a
declared signature, so it needs no θ and no interaction with either later pass.

## Why one signature-and-registry pass is complete (do not reopen)

- **No orphan (inferred-only) quotation effect reaches the runtime path.** A quotation literal
  called in the branch it is built in is spliced at compile time. Any quotation that reaches
  `lower_indirect_call` must be *materialized* (branch-join merge, or storage into an
  array/ref/struct field), and materialization is already rejected unless the quotation has a
  declared home (`different_quotations_at_join_error`, `src/check/terms.rs`).
- **No per-instantiation poly shape escapes.** `call` admits a quotation operand only when it is
  a fully-ground `PolyType::Concrete(Type::Quotation(..))` (`src/check/poly.rs:1367`); a
  variable-bearing row is rejected by `poly_op_on_variable_error` (`:1372`). By the time a
  `call` is legal its effect is concrete and present in the declared signature.

## Out of scope

The per-instantiation bundle loop and the S3k transitive fixpoint; the materialization boundary
itself; `bundle_of`, `word_ret_ty`, `intern_bundle_struct` and every lowering path.

## Invariants

- Purely additive: word-output tuples are still collected first, and IL for any program with no
  multi-output quotation is byte-identical.
- The `>= 2` filter holds at every widened site, so `lower_indirect_call`'s single-output path
  is unchanged.
- No new diagnostic and no rejection: the two guards above already reject every quotation effect
  the walk cannot reach.

## Tests

**Unit, beside `intern_output_bundles` in `src/check.rs`.** All go through an `interned_bundles`
helper that parses *without* the core prelude and runs `intern_output_bundles` directly, then
lists the `is_bundle` structs' field types; a count over a fully checked module carries the
prelude's own bundles and could not tell an over-intern from the baseline.

- `quotation_param_two_outputs_interns_a_bundle` (site 1, the repro shape)
- `poly_signature_quotation_param_interns_a_bundle` (site 4; word's own output is a single `'T`,
  so no word-level bundle can supply the tuple by coincidence)
- `struct_field_quotation_two_outputs_interns_a_bundle` (site 2)
- `array_element_quotation_two_outputs_interns_a_bundle` (site 3, via a `&[..]` parameter)
- `quotation_nested_in_a_quotation_effect_interns_a_bundle` (R2's row descent, reached through a
  struct field since a word input is gated by the nested-inside-an-effect rejection)
- `quotation_param_single_output_interns_no_bundle` (the `>= 2` filter)
- `inline_quotation_param_two_outputs_interns_no_bundle` (pins the `~[ ... ]` exclusion, so
  widening the guard to `is_quotation_type` is caught)
- `variable_bearing_poly_quotation_interns_no_bundle`: a *mixed* row (`[ i64 -- 'T i64 i64 ]`)
  interns nothing, and a `call` on a variable-bearing quotation is still rejected. The row is
  mixed deliberately: an all-variable row would also come out empty under the mistake of
  picking out the row's concrete slots.

**Goldens, `tests/phase7_slice3m.rs`** (the `tests/phase7_slice3f.rs` harness: a `Scratch`
single-file program with its own `sooth.pkg`, built and run, asserting exit `0` and exact
stdout). All four print the right values, not merely avoid the panic:

- `concrete_call_it_pushes_both_quotation_outputs` -- the repro, prints `6\n`.
- `returned_quotation_pushes_both_outputs` -- `: mk ( -- [ i64 -- i64 i64 ] )`; the word's own
  output tuple is one slot, so only descending into the slot's type finds the quotation's.
- `struct_field_quotation_pushes_both_outputs` -- `type: H run [ i64 -- i64 i64 ] ;` reached
  through `&H`, a route no word signature names.
- `polymorphic_call_it_pushes_both_quotation_outputs` -- `( 'T: Copy [ i64 -- i64 i64 ] -- 'T )`
  at two instantiations. `call_it` returns a *single* output deliberately: with `( -- 'T i64 )`
  the `'T = i64` instantiation's own return bundle is `[i64 i64]`, the exact tuple the quotation
  needs, and the golden would pass with discovery unwidened.

## Sizing

Small: one function's discovery walk plus one recursive helper; `src/check.rs` and the new
golden file only.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Widen intern_output_bundles discovery to word inputs/outputs, the struct and array registries, and poly signatures, recursing quotation effect rows to reach every ground multi-output Type::Quotation; unit tests beside it plus the end-to-end goldens", "effort": "S", "difficulty": "standard" }
  ]
}
```
