## Delivered representation

- `Type::Slice(SliceId, bool, &'static str)` (`src/ast.rs:1625`), modelled on `Type::Ref`:
  mutability inline as the classification bit, `SliceId` a `Copy` index (`ast.rs:1094`) into a
  per-program registry interned on `(element, mutable)` (`intern_slice_type`, `ast.rs:1112`).
  Display: `Slice[T]` / `!Slice[T]`.
- `IrType::Slice(SliceId)` (`src/ir/types.rs:195`), a 16-byte `{ptr, len}` aggregate,
  word-aligned. `str`'s single-word static-descriptor shape is deliberately not imitated.
- The backend spells **one shared** `:sooth.slice = { l, l }` (`backend/qbe.rs:37`), not the
  per-`SliceId` layout registry the plan imagined: element type is erased at the backend
  exactly as it is for the `Ptr` a `&T` becomes, so a per-id registry would hold N identical
  rows. `SliceId` stays a frontend/lowering discriminator with no ABI content. The type is
  emitted only when a module holds a slice (`module_has_slice`), so `tests/qbe_baseline.rs` is
  byte-identical. `sooth.slice` is unforgeable by a user struct name.
- Surface syntax is intercepted by name in `resolve_type_or_apply` ahead of every user
  registry, so `Slice` and `!Slice` are reserved in `reject_reserved_name`: a declaration under
  an intercepted name would be unreachable, not merely shadowed. The registry is threaded
  beside `refs` through the parse entry points and the whole check walk; lowering gets its own
  `Slices` registry (element `IrType` + stride), built beside `build_statics`, since a slice is
  never a field or element and takes no part in the layout DFS.

## Checker rules

- `is_ref()` answers true, which admits a `Slice` **input** past
  `check_reference_free_signature`'s input gate and makes a slice non-linear at `is_linear`.
  The output ban is untouched: the output loop tests `contains_reference` alone.
- `is_copy` splits by mutability (shared `Copy`, mutable not); `contains_reference` true;
  `find_zero_unsafe_element` names a slice zero-unsafe; `classify_capture` classifies by
  reference nature, not `Scalar`; `remap_type` rebases `SliceId` by module base.
- `qbe_abi_ty` returns the aggregate spelling. `field_load_op`/`field_store_op`/
  `scalar_size_align`/`member_ty` all **refuse** a slice, asserting R5's field-position ban
  where it is observable. `Instr::Load`/`Instr::Store` are explicit `unreachable!`: a slice
  travels by `Blit`, and a scalar move would carry one word of the two.
- `check_slice_element_gate` (`check/declarations.rs:745`) enforces the concrete-`Copy`
  element rule over the *type-spelling* route, which is where it was actually needed:
  `Slice[Slice[i64]]`, `Slice[&i64]`, `Slice[&!i64]`, `Slice[^i64]` all interned ungated and
  panicked in `slice_layout`. Wired into `check_types` and into the REPL's per-line dispatch,
  since a word signature mints the type before any `eval_*_def` runs. `slice`'s own
  construction route needs no gate: no array can hold a non-`Copy` element.
- `len` over a slice answers the carried runtime length and **consumes** its receiver
  (matching `str`, unlike the array arms which leave it), on both the mono and poly paths.
- `overlapping_projection` and `consumed_place_conflict` needed slice arms: both match
  `Type::Ref` directly for the mutability bit, so `is_ref()`'s widening never reached them and
  a live view counted as no borrow at all.

## Words

- `slice ( &[T N] -- Slice[T] )` and `subslice ( Slice[T] usize usize -- Slice[T] )`,
  compiler-known arms in the array-word family, dispatched by name, exempt from the signature
  audit as `&>` is. No new grammar. Plain word-shaped names, deliberately not sigil-shaped, so
  a later `&`-consistent projection sigil can take that namespace without moving them.
- `subslice` re-derives a fresh view from the receiver's pointer and length. References cannot
  nest, so it is never a reference-to-a-reference.
- `&>` / `&!>` accept a slice receiver and yield `&T` / `&!T`, bounds-checked against the
  runtime length via the existing `emit_oob_trap`.
- `subslice` has its **own** trap (`sooth_subslice_trap`, `emit_subslice_trap`), not the index
  message: reusing it printed `index 3 is out of bounds for length 1` where "index 3" was the
  length argument and "length 1" a computed remainder. It reports start, length, and view
  length, printed unsigned so an underflowed start reads as itself.
- Lowering had to learn reference mutability: `slice` is the first site needing it, and
  `IrType::Ptr` says nothing. `FuncBuilder::ref_mutable` carries the bit per `Value` beside
  `ref_inner`, both filled by one `record_reference` helper. Seven routes over nine seeding
  sites reach `slice`: prefix borrow (local, struct field, variant field), array-element and
  slice-element projection, owned-cell payload, declared parameter, branch `Phi`, and a
  materialized quotation's env capture. Looking the shape up by element alone is not merely
  untidy: a program holding only a mutable view has no shared row and panics.

## Borrow rules and the poly path

- `Deriv` needed no change; a view carries its receiver's derivation and region. The blind
  site was the naming arm in `check/terms.rs`, which asked `ref_parts` whether a named local is
  a reference, so a named mutable view fell to the owned-value arm and `s ... s` handed out two
  live `&!` views of one buffer. Fixed by `borrow_mutability` (`check.rs:2986`), `ref_parts`'
  sibling for sites wanting the borrow nature rather than the referent (a view has no single
  referent: it points at a run of them).
- `PolyBorrow` needed no arm: `is_reference_slot` already counts a slice, so a live view keeps
  its source borrow unpruned. What the poly path owed was walk arms, not a borrow record.
- **Rule:** a mutable view is single-use per binding inside a generic body, where the
  monomorphic path reborrows a named local. The poly walk move-tracks every non-`Copy` local,
  and a mutable view is non-`Copy`, so a generic consumer can store through a view but cannot
  read-modify-write through one. Loosening this is a change to the poly borrow model
  (slice 13's), not this slice's.
- `slice` in a poly body works off a body borrow, including over a generic *length* (which is
  the point of a view); a generic *element* is refused by name.
- Range-aware tracking stays out of scope: two simultaneously-live disjoint mutable
  sub-slices are rejected by the coarse table.

## Known gaps, recorded not fixed

- Capturing a slice **value** into a materialized quotation ICEs at `backend/qbe.rs:521`.
  Pre-existing and not slice-specific: an array or struct value panics identically, and the gap
  is the env bundle's one-word-per-capture shape. Capturing the *reference* and slicing inside
  the body works.
- A declared fully-concrete `&[i64 3]` parameter cannot be sliced or indexed in a poly body:
  it arrives as `PolyType::Concrete(Type::Ref(..))` and `poly_ref_array_parts` matches only
  `PolyType::Ref`. `&>` has the identical gap. Closing it means threading `refs` into the poly
  walk, which would move an existing diagnostic.
- `array_type_symbol`'s `arr_{idx}` and quotations' `:Q{idx}` are forgeable by a user struct
  name, and QBE silently keeps the last duplicate. Probe-confirmed latent, pre-existing; the
  fix is one shared reserved-prefix helper covering all three.
- `len_over_a_slice_answers_runtime_length` is a bare `check_src().unwrap()`, so it proves
  `len` typechecks to `usize`, not the runtime claim (which the `sum` golden carries). A name
  mismatch, not a coverage hole.
- The `Type::Variant` predicate hole slice3b flagged is adjacent to the `is_copy`/
  `contains_reference` work here but belongs to whichever slice owns the variant family.

## Testing

Goldens in `tests/phase7_slice3c.rs`: `sum_over_a_slice_noninline_prints_twentyfive` (diffed
against the length-threading twin), `recursive_divide_and_conquer_over_shared_subslices_runs`
plus its mutable twin, `slice_out_of_range_index_traps_at_runtime`,
`declared_slice_output_is_stored_reference_error`, `two_simultaneous_mutable_subslices_is_error`,
`dup_on_mutable_slice_is_error` / `dup_on_shared_slice_ok`,
`consuming_the_buffer_under_a_live_slice_is_error`,
`poly_mutable_slice_local_is_single_use`, and the R13 chain test
`mutate_innermost_hop_of_buffer_slice_subslice_element_chain_while_outer_live`.
`slice_through_a_declared_quotation_parameter_row_runs` is `inline`, not non-`inline` as first
specified: a `~[ ]` parameter can only be spliced, so the language refuses a non-`inline` word
declaring one, and the golden asserts that refusal too.

Unit tests sit beside each stage (`ast.rs`, `check/builtins.rs`, `check.rs`,
`check/captures.rs`, `check/operators.rs`, `check/declarations.rs`, `check/word_entry.rs`,
`check/audits.rs`, `check/poly.rs`, `ir/layout.rs`, `ir/types.rs`, `backend/qbe.rs`,
`check/word_families.rs`, `repl.rs`). Phase 1's output-ban claim is driven directly against
`check_reference_free_signature` and `audit_poly_reference_free_signature`, since no surface
spelling existed yet.

Every soundness arm is mutation-tested (delete the arm, a named test fails): `is_ref`,
`is_copy` and `poly_is_copy` both directions, `contains_reference`, the zero-unsafe arm,
`classify_capture`, `remap_type`, `qbe_abi_ty`, `carried_slot_bytes`, `ir_type_of`,
`member_ty`, the `&>` bounds trap, the `subslice` range trap, `len`'s consumption,
`is_aggregate`, the reserved-name checks, the interning key, `build_slices`' stride,
`overlapping_projection`, `borrow_mutability`, the mutable-receiver refusals, the poly gates
and `check_poly_slice_offset`, the trap-emission gate, and the lowering mutability lookup at
each of the nine seeding sites separately.

`cargo fmt --check`, `cargo clippy -- -D warnings`, and the full suite (1290+ tests, including
`phase7_slice3a`, `phase7_slice3b`, `qbe_baseline`) are green with the prior suites unmodified.

## Growth-signal re-check

Re-run against the four files this slice grew most: `check/poly.rs` (+471, 5456 lines),
`backend/qbe.rs` (+295, 3120), `check/word_families.rs` (+354, 2860), and
`ir/func_builder/word_families.rs` (+281, 1683). No split indicated anywhere: new arms land
inside predicates and dispatches the files already have, beside their sibling array/reference
families. `poly.rs`'s split stays deferred for the recorded reason (a `poly/diagnostics.rs`
layer split has no precedent in the checker; a `poly/eliminator.rs` split would cut the
`poly_call_term` -> `poly_eliminator_call` -> `poly_walk` mutual recursion across a file
boundary), and the second quotation consumer that was named as the trigger has not landed.

## References

- `docs/roadmap/P7/slice3c-brief.md` (probe-grounded brief; locked decisions)
- `docs/roadmap/P7-language-prereqs.md` (P7.S3c marked done, exit criterion now states the
  runtime trap rather than the stale fallible-accessor promise)
- `DESIGN.md` (storage/view length design, element `T` constraint)
- `docs/roadmap/P7/slice3a-spec.md`, `slice3b-spec.md` (forced-arm and poly-twin patterns)
