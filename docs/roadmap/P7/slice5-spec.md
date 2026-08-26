# Phase 7 Slice 5: linear array elements

**Status:** Implemented
**Created:** 2026-08-26
**Discovery:** `docs/roadmap/P7/slice5-linear-arrays-brief.md`

## Problem Statement

`[T N]` rejects a linear element for every linear type (`check_array_element_gate`, diagnosed by `fill_of_linear_element_error`), even though the same type as a struct field builds fine. A linear struct is storable but a *collection* of them is not — the one gap left in the linear spine reaching arrays, re-observed as somebody else's blocker across four prior slices. Closing it needs three things together, not a predicate flip: a construction path that does not replicate a linear value (`fill` always stores the same SSA value into every slot), a disposal path that does not exist (`emit_drop`'s `IrType::Array` arm is `unreachable!`), and a decision about the partially-initialized window during construction.

## Requirements

- **R1.** A new builtin array word family `tabulate ( usize ~[ -- T ] -- [T N] )` allocates an array and, for each index `0..N`, splices the quotation to produce a fresh `T` and stores it into that slot. The quotation is inline-spliced: the checker arm calls `check_literal_against_declared_effect` directly with a synthesized inline `QuotEffect { inputs: [], outputs: [T] }` — the same function the ordinary call-check path uses for library words like `times`, but called directly from the word-family arm rather than reached through generic call resolution (word families bypass that path). The IR lowering splices the quotation body inside the loop via `lower_terms` (the same function `call`-of-literal uses to inline a quotation body in place), so no value persists across iterations and a linear `T` is safe: each slot gets a distinct, freshly-constructed value, never a replicated one. `tabulate` is a word family (like `fill`/`len`), not a library word, because it must allocate the array and manage the raw-storage boundary, which is IR-level work `times`-as-library-code does not do.
- **R2.** `tabulate`'s checker arm does **not** call `check_array_element_gate` at all — the gate's `is_copy` check is irrelevant because the element is freshly produced by the quotation each iteration, never replicated. The quotation's output type (checked by `check_literal_against_declared_effect` against the declared `~[ -- T ]` effect) *is* the element type; a type mismatch surfaces through the existing `literal_effect_mismatch_error` diagnostic path, not a new one. `fill`'s call site continues to call `check_array_element_gate` and rejects a linear element **unless** R3's nullary-variant relaxation applies.
- **R3.** `fill` admits a linear element when the seed value is statically known to be a nullary enum variant (a variant with no payload). The checker tracks this via a new `Slot.variant_idx: Option<u32>` field, set when a nullary variant constructor pushes its value and cleared like `int_val` (by any operator/conversion/word call or branch merge — no folding through non-identity ops). `check_array_element_gate`'s call site for `fill` reads the seed slot's `variant_idx`; a linear element is admitted iff the seed is `Some(_)` (a known nullary variant), still rejected otherwise. This does not touch `is_copy` — the relaxation is an additional gate condition, not a modification of the copy check.
- **R4.** The `[Type; Count]` array constructor is deleted: the parser production, the `array_ctor_ahead` lookahead, the `TermKind::ArrayCtor` checker path, and the IR lowering are all removed. Its `zero_safety` flag on `check_array_element_gate` is removed along with it (the flag was ctor-only; `fill`'s call site never set it). The `linear array elements are not supported yet` diagnostic (`fill_of_linear_element_error`) is deleted, not reworded — its callers (the ctor and `fill`) either no longer exist (ctor) or now have a real admit path (`fill`, R3) with its own rejection message for the remaining non-nullary-linear case.
- **R5.** `examples/array_ctor.sth`'s 6 `[Type; Count]` usages migrate to `fill` mechanically: `[i64; 10]` → `0 10 fill`, `[i8; 10]` → `0 >i8 10 fill`, `[Bool; 4]` → `False 4 fill`, and three `[i64; 4]` usages → `0 4 fill`, preserving the example's existing purpose (the store loop overwriting dirty stack residue).
- **R6.** A `synthesize_array_destructor` (mirroring `synthesize_struct_destructor`) is added to `synthesize_aggregate_destructors` for every linear array shape reachable from the program. It emits a constant-trip-count IR loop over `0..N` that loads each element and calls `emit_drop` on it — no allocation to free, since arrays are stack-allocated (`Instr::Alloc`). A new `array_drop_symbol` (mirroring `struct_drop_symbol`/`enum_drop_symbol`) names the synthesized function, keyed on `(ArrayId, drop_generation)` the same way the existing symbols are.
- **R7.** `emit_drop`'s `IrType::Array` arm calls the synthesized destructor via its `array_drop_symbol` when `self.arrays.layouts[id.index()].is_linear`, replacing the `unreachable!`. The non-linear arm (`_ => {}`, no-op) is unchanged.
- **R8.** The quotation passed to `tabulate` has effect `~[ -- T ]` — it may only produce, never consume the slot it is filling. This is enforced by `check_literal_against_declared_effect` with an empty `eff.inputs` list (the quotation gets no inputs from the caller), so a quotation body cannot call `drop` on the element under construction; there is no new diagnostic to write. If the quotation traps or aborts mid-loop, the process exits before the partially-built array is ever observed as a value — the same behavior `fill` already has today for a trapping seed construction. No new type-system concept models the partially-initialized window; it exists only in the IR (raw `Instr::Alloc` storage, written in a loop, surfacing as a `Type::Array` value only after the loop completes and `push dst` runs), exactly as it already does inside `fill`.
- **R9.** (NFR, parity) Every existing non-linear use of `fill`, `len`, and array types compiles and runs unchanged; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` stays green.
- **R10.** (NFR, diagnostics) A `fill` call on a linear, non-nullary-variant element (the case R3 does not cover — e.g. `type: Spy s str ; ... Spy@"x" 3 fill`) is a located error distinct from the deleted `fill_of_linear_element_error`, naming the element type and that `tabulate` is the construction path for distinct linear values.
- **R11.** (NFR, golden) A golden test demonstrates `tabulate` building a linear array (e.g. `[Spy N]` where each `Spy` wraps a distinct string), the array being consumed element-wise, and — separately — the array being dropped whole, exercising R6/R7's synthesized destructor.
- **R12.** (NFR, golden) A golden test demonstrates `None 3 fill` producing `[Option[Spy] 3]` (a linear-enum array via the nullary-variant seed) and that array being dropped, disposing the (empty) `None` slots as a no-payload discriminant write — no leaked linear data, since there is none in a `None` slot.
- **R13.** (NFR, golden) A golden test demonstrates that `fill`ing a linear array with a non-nullary seed is rejected with R10's located error, and that `[Type; Count]` no longer parses (a located "unknown term" or equivalent parse error, not a silent fallthrough to quotation parsing).

## Success Criteria

- `type: Arr xs [Spy 2] ;` builds when `Arr` is constructed via `tabulate`, and the array disposes both `Spy` elements exactly once when `Arr` is dropped.
- `None 3 fill` builds an `[Option['T] 3]` array of `None` sentinels for a linear `'T`, without the checker replicating a linear payload (there is none to replicate).
- `[Type; Count]` is gone: no parser production, no `array_ctor_ahead` lookahead, no `TermKind::ArrayCtor`, no lowering, and the migrated `examples/array_ctor.sth` builds and runs identically via `fill`.
- `fill`ing a linear array with a data-carrying (non-nullary) seed is a located error naming `tabulate` as the alternative.
- A linear array dropped via any path (scope exit, explicit `drop`, as a struct field) disposes every element exactly once via the synthesized array destructor.
- `examples/array_ctor.sth` and the rest of the example/lib corpus compile and run unchanged; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
- Each new function (`tabulate`'s checker/IR word-family handlers, the nullary-variant seed gate, `synthesize_array_destructor`, `array_drop_symbol`) has unit tests beside it: a happy path plus at least one error/edge case.

## Scope & Boundaries

**In scope:**

- The `tabulate` word family (checker signature + IR lowering, mirroring `fill`'s existing loop shape with the store swapped for a spliced quotation call).
- The nullary-variant seed relaxation on `fill`'s element gate, including the `Slot.variant_idx` field and its propagation/clearing rules.
- Deleting `[Type; Count]` end to end (parser, checker, IR, diagnostic) and migrating its one example usage.
- The synthesized array destructor and its wiring into `emit_drop`.
- The four goldens above and unit tests for every new/changed function.

**Out of scope (per the brief):**

- A dynamically-sized or growable array (a library `Vec`, needing the struct-header length variable **P7.S3n** named and did not land).
- A linear element reached through a `Slice[T]` view (a view does not own what it points at).
- Zero-cost reservation without a sentinel (deferred to P11, pending a concrete RT consumer).
- A `Default` trait (would be replication under another name).
- The `fill` memset-when-all-zero-seed lowering optimization (open question 4, resolved below: deferred, not required for this slice's exit).

## Design Decisions & Rationale

**Ruling on open question 1 (`Slot` variant-identity tracking): a new `Slot` field, not a narrower path.** A mechanism scoped only to the `fill` check path would need its own way to trace a seed value back to the variant constructor that produced it — which is exactly what `Slot` already exists to carry forward (see `int_val`, tracked the same way for the unrelated `fill`-count/bounds-check purpose). `Slot` is `Copy` and the field is a single `Option<u32>`, the same shape and cost as `int_val`; threading provenance through a side channel instead would duplicate `Slot`'s existing propagation/clearing logic rather than reuse it. The field is set only when a# Phase 7 Slice 5: linear array elements

**Status:** Implemented
**Created:** 2026-08-26
**Discovery:** `docs/roadmap/P7/slice5-linear-arrays-brief.md`

## Problem Statement

`[T N]` rejects a linear element for every linear type (`check_array_element_gate`, `fill_of_linear_element_error`), even though the same type as a struct field builds fine. A linear struct is storable but a *collection* of them is not — the one gap left in the linear spine reaching arrays. Closing it needs three things together, not a predicate flip: a construction path that does not replicate a linear value (`fill` always stores the same SSA value into every slot), a disposal path that does not exist (`emit_drop`'s `IrType::Array` arm is `unreachable!`), and a decision about the partially-initialized window during construction.

## Requirements

- **R1.** A new builtin array word family `tabulate ( usize ~[ -- T ] -- [T N] )` allocates an array and, for each index `0..N`, splices the quotation to produce a fresh `T` and stores it into that slot. The quotation is inline-spliced: the checker arm calls `check_literal_against_declared_effect` directly with a synthesized inline `QuotEffect { inputs: [], outputs: [T] }`. The IR lowering splices the quotation body inside the loop via `lower_terms`, so no value persists across iterations and a linear `T` is safe: each slot gets a distinct, freshly-constructed value. `tabulate` is a word family (like `fill`/`len`) because it must allocate the array and manage the raw-storage boundary, which is IR-level work `times`-as-library-code does not do.
- **R2.** `tabulate`'s checker arm does **not** call `check_array_element_gate` at all — the gate's `is_copy` check is irrelevant because the element is freshly produced by the quotation each iteration, never replicated. A type mismatch surfaces through the existing `literal_effect_mismatch_error` diagnostic path. `fill`'s call site continues to call `check_array_element_gate` and rejects a linear element **unless** R3's nullary-variant relaxation applies.
- **R3.** `fill` admits a linear element when the seed value is statically known to be a nullary enum variant (a variant with no payload). The checker tracks this via a new `Slot.variant_idx: Option<u32>` field, set when a nullary variant constructor pushes its value and cleared like `int_val` (by any operator/conversion/word call or branch merge — no folding through non-identity ops). A linear element is admitted iff the seed is `Some(_)` (a known nullary variant), still rejected otherwise. This is an additional gate condition, not a modification of the copy check.
- **R4.** The `[Type; Count]` array constructor is deleted: the parser production (`parse_array_ctor_term`, `array_ctor_ahead` lookahead), the `TermKind::ArrayCtor` checker path, and the IR lowering are all removed. Its `zero_safety` flag on `check_array_element_gate` is removed along with it. The `fill_of_linear_element_error` diagnostic is deleted, not reworded.
- **R5.** `examples/array_ctor.sth`'s 6 `[Type; Count]` usages migrate to `fill` mechanically, preserving the example's existing purpose (the store loop overwriting dirty stack residue).
- **R6.** A `synthesize_array_destructor` (mirroring `synthesize_struct_destructor`) is added for every linear array shape reachable from the program. It emits a constant-trip-count IR loop over `0..N` that loads each element and calls `emit_drop` on it — no allocation to free, since arrays are stack-allocated (`Instr::Alloc`). A new `array_drop_symbol` (mirroring `struct_drop_symbol`/`enum_drop_symbol`) names the synthesized function, keyed on `(ArrayId, drop_generation)`.
- **R7.** `emit_drop`'s `IrType::Array` arm calls the synthesized destructor via its `array_drop_symbol` when the array is linear, replacing the `unreachable!`. The non-linear no-op arm is unchanged.
- **R8.** The quotation passed to `tabulate` has effect `~[ -- T ]` — it may only produce, never consume the slot it is filling, enforced by the empty `eff.inputs` list. If the quotation traps or aborts mid-loop, the process exits before the partially-built array is ever observed as a value — the same behavior `fill` already has today. No new type-system concept models the partially-initialized window; it exists only in the IR, exactly as it already does inside `fill`.
- **R9.** (NFR, parity) Every existing non-linear use of `fill`, `len`, and array types compiles and runs unchanged; full green.
- **R10.** (NFR, diagnostics) A `fill` call on a linear, non-nullary-variant element is a located error distinct from the deleted `fill_of_linear_element_error`, naming the element type and that `tabulate` is the construction path for distinct linear values.
- **R11.** (NFR, golden) A golden test demonstrates `tabulate` building a linear array, the array being consumed element-wise, and — separately — the array being dropped whole, exercising R6/R7's synthesized destructor.
- **R12.** (NFR, golden) A golden test demonstrates `None 3 fill` producing `[Option[Spy] 3]` (a linear-enum array via the nullary-variant seed) and that array being dropped, disposing the (empty) `None` slots as a no-payload discriminant write — no leaked linear data.
- **R13.** (NFR, golden) A golden test demonstrates that `fill`ing a linear array with a non-nullary seed is rejected with R10's located error, and that `[Type; Count]` no longer parses (a located parse error, not a silent fallthrough).

## Success Criteria

- `type: Arr xs [Spy 2] ;` builds when `Arr` is constructed via `tabulate`, and the array disposes both `Spy` elements exactly once when dropped.
- `None 3 fill` builds an `[Option['T] 3]` array of `None` sentinels for a linear `'T`, without replicating a linear payload.
- `[Type; Count]` is gone: no parser production, no `TermKind::ArrayCtor`, no lowering, and the migrated `examples/array_ctor.sth` builds and runs identically via `fill`.
- `fill`ing a linear array with a data-carrying (non-nullary) seed is a located error naming `tabulate` as the alternative.
- A linear array dropped via any path (scope exit, explicit `drop`, as a struct field) disposes every element exactly once via the synthesized array destructor.
- `examples/array_ctor.sth` and the rest of the example/lib corpus compile and run unchanged; full green.
- Each new function has unit tests beside it: a happy path plus at least one error/edge case.

## Scope & Boundaries

**In scope:** the `tabulate` word family (checker + IR lowering); the nullary-variant seed relaxation on `fill`'s element gate; deleting `[Type; Count]` end to end and migrating its one example usage; the synthesized array destructor and its wiring into `emit_drop`; goldens and unit tests for every new/changed function.

**Out of scope (per the brief):**

- A dynamically-sized or growable array (a library `Vec`, needing the struct-header length variable P7.S3 named and did not land).
- A linear element reached through a `Slice[T]` view (a view does not own what it points at).
- Zero-cost reservation without a sentinel (deferred to P11, pending a concrete RT consumer).
- A `Default` trait (would be replication under another name).
- The `fill` memset-when-all-zero-seed lowering optimization (deferred, not required for this slice's exit).

## Design Decisions & Rationale

**Open question 1 — `Slot` variant-identity tracking: a new `Slot` field, not a narrower path.** A mechanism scoped only to the `fill` check path would need its own way to trace a seed value back to the variant constructor — which is exactly what `Slot` already exists to carry forward (see `int_val`, tracked the same way for the unrelated `fill`-count/bounds-check purpose). `Slot` is `Copy` and the field is a single `Option<u32>`, the same shape and cost as `int_val`. The field is set only when a nullary variant constructor pushes a value and cleared by the same rule `int_val` uses: any operator, conversion, word call, or branch merge clears it, since only a moved-verbatim duplicate (a shuffle) preserves known provenance.

**Open question 2 — `tabulate`: word family vs. library word: word family.** `tabulate` must allocate array storage and manage the raw-to-value boundary crossing (`alloc_array` → store loop → `push dst`), which is IR-level work no library word can express — `fill` is a word family for the identical reason. `times`'s library-word status is not evidence against this: `times` never allocates, it only loops.

**Open question 3 — destructor: loop vs. unroll: a constant-trip-count IR loop.** It matches `fill`'s existing loop pattern exactly, keeps the destructor's size independent of `N`, and is simpler to generate and reason about than emitting `N` unrolled load-and-drop instructions. QBE is free to unroll a small constant-trip loop itself; this slice does not hand-unroll in the frontend.

**Open question 4 — `fill` memset optimization: deferred to a follow-up.** The optimization preserves the performance the deleted `[Type; Count]` path had, but is not required for correctness or for the linear-element exit criterion. Bundling it would mix a performance optimization into a slice whose exit is about admitting linear elements. Tracked as a follow-up once a concrete performance need is demonstrated.

## Open Questions

None outstanding — the four questions the brief flagged are resolved above.

## Implementation

| Area | Commit | Key files |
|------|--------|-----------|
| `tabulate` word family: checker arm + IR lowering | `6f64cc66` | `examples/tabulate.sth`, `src/check/terms.rs`, `src/check/word_families.rs`, `src/ir/func_builder/calls.rs`, `src/ir/func_builder/word_families.rs` |
| Nullary-variant `fill` relaxation: `Slot.variant_idx`, non-nullary-linear-seed rejection | `e823caf2` | `src/check.rs`, `src/check/engine.rs`, `src/check/terms.rs`, `src/check/word_families.rs`, `tests/phase0.rs` |
| Delete `[Type; Count]` end to end; migrate `examples/array_ctor.sth` | `36316f45` | `examples/array_ctor.sth`, `src/ast.rs`, `src/check.rs`, `src/check/declarations.rs`, `src/check/engine.rs`, `src/check/poly.rs`, `src/check/terms.rs`, `src/check/word_families.rs` |
| Array destructor synthesis + `emit_drop` wiring | `1df5f86d` | `src/ir/destructors.rs`, `src/ir/func_builder/quotation.rs`, `src/ir/func_builder/word_families.rs`, `src/ir/layout.rs`, `src/ir.rs`, `src/check/word_families.rs`, `src/repl.rs`, `tests/phase7_slice5_array_drop.rs` |
| Review-feedback fixes (cycle 1) | `959eb54f` | `src/check/audits.rs`, `src/check/declarations.rs`, `src/check/poly.rs`, `src/check/word_families.rs`, `src/driver.rs`, `src/repl.rs`, `tests/phase0.rs`, `docs/check-modularisation-brief.md` |
