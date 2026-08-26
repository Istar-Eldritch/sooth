# Phase 7 Slice 5: linear array elements

**Status:** Draft
**Created:** 2026-08-26
**Discovery:** `docs/roadmap/P7/slice5-linear-arrays-brief.md`

## Problem Statement

`[T N]` rejects a linear element for every linear type (`check_array_element_gate`,
`src/check.rs:510`, diagnosed by `fill_of_linear_element_error`, `src/check.rs:3224`),
even though the same type as a struct field builds fine. A linear struct is storable
but a *collection* of them is not — the one gap left in the linear spine reaching
arrays, re-observed as somebody else's blocker across four prior slices. Closing it
needs three things together, not a predicate flip: a construction path that does not
replicate a linear value (`fill` always stores the same SSA value into every slot),
a disposal path that does not exist (`emit_drop`'s `IrType::Array` arm is
`unreachable!`, `src/ir/func_builder/quotation.rs:412`), and a decision about the
partially-initialized window during construction.

## Requirements

- **R1.** A new builtin array word family `tabulate ( usize ~[ -- T ] -- [T N] )`
  allocates an array and, for each index `0..N`, splices the quotation to produce a
  fresh `T` and stores it into that slot. The quotation is inline-spliced: the
  checker arm calls `check_literal_against_declared_effect` (`src/check.rs:2228`)
  directly with a synthesized inline `QuotEffect { inputs: [], outputs: [T] }` — the
  same function the ordinary call-check path uses for library words like `times`,
  but called directly from the word-family arm rather than reached through generic
  call resolution (word families bypass that path; `check_array_word`,
  `src/check/word_families.rs:757`, intercepts by name before the call checker).
  The IR lowering splices the quotation body inside the loop via `lower_terms`
  (the same function `call`-of-literal uses at `src/ir/func_builder/calls.rs:362` to
  inline a quotation body in place), so no value persists across iterations and a
  linear `T` is safe: each slot gets a distinct, freshly-constructed value, never
  a replicated one. `tabulate` is a word family (like `fill`/`len`), not a library
  word, because it must allocate the array and manage the raw-storage boundary,
  which is IR-level work `times`-as-library-code does not do.
- **R2.** `tabulate`'s checker arm does **not** call `check_array_element_gate`
  (`src/check.rs:510`) at all — the gate's `is_copy` check is irrelevant because
  the element is freshly produced by the quotation each iteration, never
  replicated. The quotation's output type (checked by
  `check_literal_against_declared_effect` against the declared `~[ -- T ]` effect)
  *is* the element type; a type mismatch surfaces through the existing
  `literal_effect_mismatch_error` diagnostic path, not a new one. `fill`'s call
  site continues to call `check_array_element_gate` and rejects a linear element
  **unless** R3's nullary-variant relaxation applies.
- **R3.** `fill` admits a linear element when the seed value is statically known to be
  a nullary enum variant (a variant with no payload). The checker tracks this via a
  new `Slot.variant_idx: Option<u32>` field (`src/check.rs:278`), set when a nullary
  variant constructor pushes its value and cleared like `int_val` (by any
  operator/conversion/word call or branch merge — no folding through non-identity
  ops). `check_array_element_gate`'s call site for `fill` reads the seed slot's
  `variant_idx`; a linear element is admitted iff the seed is `Some(_)` (a known
  nullary variant), still rejected otherwise. This does not touch `is_copy` — the
  relaxation is an additional gate condition, not a modification of the copy check.
- **R4.** The `[Type; Count]` array constructor is deleted: the parser production
  (`parse_array_ctor_term`, `src/parser.rs:4021`), the `array_ctor_ahead` lookahead
  (`src/parser.rs:3990`), the `TermKind::ArrayCtor` checker path
  (`src/check/terms.rs:1033`), and the IR lowering (`src/ir/func_builder/calls.rs:74`)
  are all removed. Its `zero_safety` flag on `check_array_element_gate` is removed
  along with it (the flag was ctor-only; `fill`'s call site never set it). The
  `linear array elements are not supported yet` diagnostic
  (`fill_of_linear_element_error`, `src/check.rs:3224`) is deleted, not reworded —
  its callers (the ctor and `fill`) either no longer exist (ctor) or now have a real
  admit path (`fill`, R3) with its own rejection message for the remaining
  non-nullary-linear case.
- **R5.** `examples/array_ctor.sth`'s 6 `[Type; Count]` usages migrate to `fill`
  mechanically: `[i64; 10]` → `0 10 fill` (line 26), `[i8; 10]` → `0 >i8 10 fill`
  (line 33), `[Bool; 4]` → `False 4 fill` (line 40), and three `[i64; 4]` usages →
  `0 4 fill` (lines 50, 57, 64), preserving the example's existing purpose (the
  store loop overwriting dirty stack residue).
- **R6.** A `synthesize_array_destructor` (mirroring `synthesize_struct_destructor`,
  `src/ir/destructors.rs:310`) is added to `synthesize_aggregate_destructors`
  (`src/ir/destructors.rs:37`) for every linear array shape reachable from the
  program. It emits a constant-trip-count IR loop over `0..N` that loads each element
  and calls `emit_drop` on it — no allocation to free, since arrays are
  stack-allocated (`Instr::Alloc`). A new `array_drop_symbol` (mirroring
  `struct_drop_symbol`/`enum_drop_symbol`, `src/ir/layout.rs:130-148`) names the
  synthesized function, keyed on `(ArrayId, drop_generation)` the same way the
  existing symbols are.
- **R7.** `emit_drop`'s `IrType::Array` arm (`src/ir/func_builder/quotation.rs:412`,
  calls the synthesized destructor via its `array_drop_symbol` when
  `self.arrays.layouts[id.index()].is_linear`, replacing the `unreachable!`. The
  non-linear arm (`_ => {}`, no-op) is unchanged.
- **R8.** The quotation passed to `tabulate` has effect `~[ -- T ]` — it may only
  produce, never consume the slot it is filling. This is enforced by
  `check_literal_against_declared_effect` with an empty `eff.inputs` list (the
  quotation gets no inputs from the caller), so a quotation body cannot call `drop`
  on the element under construction; there is no new diagnostic to write. If the quotation traps or aborts mid-loop, the process exits before the
  partially-built array is ever observed as a value — the same behavior `fill`
  already has today for a trapping seed construction. No new type-system concept
  models the partially-initialized window; it exists only in the IR (raw
  `Instr::Alloc` storage, written in a loop, surfacing as a `Type::Array` value only
  after the loop completes and `push dst` runs), exactly as it already does inside
  `fill`.
- **R9.** (NFR, parity) Every existing non-linear use of `fill`, `len`, and array
  types compiles and runs unchanged; `cargo fmt --check && cargo clippy -- -D
  warnings && cargo test` stays green.
- **R10.** (NFR, diagnostics) A `fill` call on a linear, non-nullary-variant element
  (the case R3 does not cover — e.g. `type: Spy s str ; ... Spy@"x" 3 fill`) is a
  located error distinct from the deleted `fill_of_linear_element_error`, naming the
  element type and that `tabulate` is the construction path for distinct linear
  values.
- **R11.** (NFR, golden) A golden test demonstrates `tabulate` building a linear
  array (e.g. `[Spy N]` where each `Spy` wraps a distinct string), the array being
  consumed element-wise, and — separately — the array being dropped whole, exercising
  R6/R7's synthesized destructor.
- **R12.** (NFR, golden) A golden test demonstrates `None 3 fill` producing
  `[Option[Spy] 3]` (a linear-enum array via the nullary-variant seed) and that array
  being dropped, disposing the (empty) `None` slots as a no-payload discriminant
  write — no leaked linear data, since there is none in a `None` slot.
- **R13.** (NFR, golden) A golden test demonstrates that `fill`ing a linear array
  with a non-nullary seed is rejected with R10's located error, and that
  `[Type; Count]` no longer parses (a located "unknown term" or equivalent parse
  error, not a silent fallthrough to quotation parsing).

## Success Criteria

- `type: Arr xs [Spy 2] ;` builds when `Arr` is constructed via `tabulate`, and the
  array disposes both `Spy` elements exactly once when `Arr` is dropped.
- `None 3 fill` builds an `[Option['T] 3]` array of `None` sentinels for a linear
  `'T`, without the checker replicating a linear payload (there is none to
  replicate).
- `[Type; Count]` is gone: no parser production, no `array_ctor_ahead` lookahead,
  no `TermKind::ArrayCtor`, no lowering, and the migrated `examples/array_ctor.sth`
  builds and runs identically via `fill`.
- `fill`ing a linear array with a data-carrying (non-nullary) seed is a located
  error naming `tabulate` as the alternative.
- A linear array dropped via any path (scope exit, explicit `drop`, as a struct
  field) disposes every element exactly once via the synthesized array destructor.
- `examples/array_ctor.sth` and the rest of the example/lib corpus compile and run
  unchanged; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is
  green.
- Each new function (`tabulate`'s checker/IR word-family handlers, the
  nullary-variant seed gate, `synthesize_array_destructor`, `array_drop_symbol`) has
  unit tests beside it: a happy path plus at least one error/edge case.

## Scope & Boundaries

**In scope:**

- The `tabulate` word family (checker signature + IR lowering, mirroring `fill`'s
  existing loop shape with the store swapped for a spliced quotation call).
- The nullary-variant seed relaxation on `fill`'s element gate, including the
  `Slot.variant_idx` field and its propagation/clearing rules.
- Deleting `[Type; Count]` end to end (parser, checker, IR, diagnostic) and
  migrating its one example usage.
- The synthesized array destructor and its wiring into `emit_drop`.
- The four goldens above and unit tests for every new/changed function.

**Out of scope (per the brief):**

- A dynamically-sized or growable array (a library `Vec`, needing the struct-header
  length variable **P7.S3n** named and did not land).
- A linear element reached through a `Slice[T]` view (a view does not own what it
  points at).
- Zero-cost reservation without a sentinel (deferred to P11, pending a concrete RT
  consumer).
- A `Default` trait (would be replication under another name).
- The `fill` memset-when-all-zero-seed lowering optimization (open question 4,
  resolved below: deferred, not required for this slice's exit).

## Design Decisions & Rationale

**Ruling on open question 1 (`Slot` variant-identity tracking): a new `Slot` field,
not a narrower path.** A mechanism scoped only to the `fill` check path would need
its own way to trace a seed value back to the variant constructor that produced it —
which is exactly what `Slot` already exists to carry forward (see `int_val`, tracked
the same way for the unrelated `fill`-count/bounds-check purpose). `Slot` is `Copy`
and the field is a single `Option<u32>`, the same shape and cost as `int_val`;
threading provenance through a side channel instead would duplicate `Slot`'s
existing propagation/clearing logic rather than reuse it. The field is set only when
a nullary variant constructor pushes a value (mirroring where `int_val` is set for
an `IntLit`) and cleared by the same rule `int_val` uses: any operator, conversion,
word call, or branch merge clears it, since only a moved-verbatim duplicate (a
shuffle) preserves known provenance.

**Ruling on open question 2 (`tabulate`: word family vs. library word): word
family.** `tabulate` must allocate array storage and manage the raw-to-value
boundary crossing (`alloc_array` → store loop → `push dst`), which is IR-level work
no library word can express — `fill` is a word family for the identical reason. The quotation splicing reuses the same checker function
(`check_literal_against_declared_effect`, `src/check.rs:2228`) and the same IR
splice (`lower_terms`, as `call`-of-literal uses at `src/ir/func_builder/calls.rs:362`)
that library words like `times` use through the ordinary call path — but the
attachment point differs: `tabulate`'s word-family arm calls these directly instead
of being reached through generic call resolution. `times`'s library-word status is
not evidence against this: `times` never allocates, it only loops.

**Ruling on open question 3 (destructor: loop vs. unroll): a constant-trip-count IR
loop.** It matches `fill`'s existing loop pattern exactly (same `begin_loop`/
`elem_addr`/back-edge shape `synthesize_struct_destructor` also reuses via
`FuncBuilder`'s loop primitives), keeps the destructor's size independent of `N`,
and is simpler to generate and to reason about than emitting `N` unrolled
load-and-drop instructions. QBE is free to unroll a small constant-trip loop itself
if it chooses; this slice does not hand-unroll in the frontend.

**Ruling on open question 4 (`fill` memset optimization): deferred to a follow-up.**
The optimization (lowering `fill` to a byte-granular memset when the seed's bit
pattern is provably all-zero) preserves the performance the deleted `[Type; Count]`
path had, but is not required for correctness or for the linear-element exit
criterion — every `fill` call, zero-seed or not, already lowers correctly through
the existing store loop. Bundling it into this slice would mix a performance
optimization into a slice whose exit is about admitting linear elements. Tracked as
a follow-up once a concrete performance need is demonstrated (the same "defer until
a real consumer" discipline the brief applies to zero-cost reservation).

**Difficulty justification: Phases 1 and 2 are `hard`; Phases 3 and 4 are
`standard`.** Phase 1 is `hard` because no existing word family consumes a
quotation — every `check_array_word` arm actively rejects quotations (`:859`,
`:862`, `:902`). `tabulate`'s checker arm calling
`check_literal_against_declared_effect` directly is an ambiguous integration point
with no precedent to copy. Phase 2 is `hard` because `Slot.variant_idx` has a
silent-soundness risk: a missed clear-site (any operator or word call that
transforms the seed but doesn't clear `variant_idx`) would falsely admit a linear
non-nullary seed, replicating a linear value. The ~41 `Slot` construction sites all
need the new field, and a single miss is a soundness hole, not a compile error.
Phase 3 (deleting dead code paths) and Phase 4 (mirroring an existing destructor
pattern) are `standard` — they follow established patterns with no ambiguous
integration.

## Open Questions

None outstanding — the four questions the brief flagged are resolved above.

## Implementation

### Phase 1 — `tabulate` word family (R1, R2, R8)

**Scope.**

- `src/check/word_families.rs:757` (`check_array_word`) — new `"tabulate"` arm:
  accepts a quotation operand (unlike `fill` which rejects at `:859`), pops count
  and quotation from the stack, calls `check_literal_against_declared_effect`
  (`src/check.rs:2228`) directly with a synthesized inline `QuotEffect { inputs:
  [], outputs: [element_ty] }` to type-check the quotation body, and does NOT call
  `check_array_element_gate` (the element is freshly produced, not replicated).
- `src/ir/func_builder/calls.rs:580` — add `"tabulate"` to the
  `"fill" | "slice" | "subslice"` dispatch arm.
- `src/ir/func_builder/word_families.rs:386` (`lower_array_word`) — new `"tabulate"`
  arm: pop count and quotation, `alloc_array`, `begin_loop`, splice quotation body
  via `lower_terms` (same pattern as `call`-of-literal at `calls.rs:362`),
  `store_elem` the result, back-edge, `finalize_loop`, `push dst`. Update the
  fallback `unreachable!` at `:529` to include `tabulate`.

**Out of bounds.** `fill`'s own gate/lowering (phase 2), the `[Type; Count]`
deletion (phase 3), the array destructor (phase 4).

**Entry.** None; current `main` is green.

**Exit.** `tabulate` type-checks and IR-lowers for a linear element type
(verified by a compilation test: a `.sth` file using `tabulate` to build a linear
  array compiles successfully); unit tests for the checker arm (accepts a
  `~[ -- T ]` quotation, rejects a mismatched effect, admits linear `T`) and the
  IR lowering (allocates, loops, splices per iteration, stores); `cargo fmt
  --check && cargo clippy -- -D warnings && cargo test` green. The R11 golden is
  deferred to Phase 4 — a linear array cannot be dropped or consumed element-wise
  until the destructor (R6/R7) and a by-value linear-element-read exist, neither of
  which Phase 1 provides.

### Phase 2 — nullary-variant `fill` relaxation (R3, R10, R12, R13 partial)

**Scope.** `src/check.rs:278` (`Slot.variant_idx: Option<u32>` field and its
set/clear/shuffle-preserve rules, mirroring `int_val`). The `Slot` struct is
constructed by explicit field literal at ~41 sites across `src/check.rs` and
`src/check/*.rs` (not `..Default`); every site needs the new field added.
Representative sites: the `fill` arm's output slot (`word_families.rs:898`), the
`len` arm's output (`:935`), `Slot::computed`/`Slot::derived` constructors used
throughout. The nullary-variant constructor call site that sets `variant_idx`,
`check_array_element_gate`'s `fill` call site at `src/check/word_families.rs:893`
(reads `variant_idx` on the seed slot), a new located diagnostic for the
non-nullary-linear-seed rejection (R10).

**Out of bounds.** `is_copy` itself (unchanged), `tabulate` (phase 1, already
landed), the ctor deletion (phase 3), the destructor (phase 4).

**Entry.** Phase 1 landed and green.

**Exit.** `None 3 fill` builds a linear-enum array from a nullary seed; a
non-nullary linear seed to `fill` is rejected with R10's new message (not the old
`fill_of_linear_element_error`, which phase 3 deletes); R12's golden (construction
half) and R13's rejection golden pass; unit tests for `variant_idx` propagation
(set, cleared-by-op, preserved-by-shuffle) and the gate's read; full green.

### Phase 3 — delete `[Type; Count]` (R4, R5, R13 remainder)

**Scope.** `src/parser.rs` (`parse_array_ctor_term` at `:4021`, `array_ctor_ahead`
at `:3990`), `src/check/terms.rs` (`TermKind::ArrayCtor` handling at `:1033`),
`src/ir/func_builder/calls.rs` (the ctor lowering at `:74`), `src/check.rs`
(`fill_of_linear_element_error` at `:3224` and the `zero_safety` flag on
`check_array_element_gate`), `examples/array_ctor.sth` (R5's migration).

**Out of bounds.** `fill`'s relaxed gate logic (phase 2, already landed), the
destructor (phase 4).

**Entry.** Phase 2 landed and green.

**Exit.** `[Type; Count]` does not parse (R13's parse-error golden); no dead code
remains from the ctor's parser/checker/IR paths (clippy would flag an unused
`zero_safety`-shaped remnant if any survived); `examples/array_ctor.sth` builds and
runs identically post-migration; full green.

### Phase 4 — array destructor synthesis (R6, R7, R9, R11 remainder, R12 remainder)

**Scope.** `src/ir/destructors.rs` (`synthesize_array_destructor`, wired into
`synthesize_aggregate_destructors` at `:37`), `src/ir/layout.rs`
(`array_drop_symbol`, mirroring `struct_drop_symbol`/`enum_drop_symbol` at
`:130-148`), `src/ir/func_builder/quotation.rs:412` (`emit_drop`'s `IrType::Array`
arm).

**Out of bounds.** Anything in `src/check.rs` or the parser — if phase 4 needs a
checker or parser edit, phases 1-3 left the array-element gate wrong and that is
the finding to report.

**Entry.** Phase 3 landed and green.

**Exit.** A linear array (built via `tabulate`) disposes every element exactly once
on drop, via any drop path (scope exit, explicit `drop`, nested as a struct field);
the non-linear `_ => {}` no-op arm is unchanged and covered by an existing
non-regression check; R11's and R12's goldens are complete (construction +
disposal); a mutation test deleting the new destructor-dispatch arm (reverting to
`unreachable!`) makes R11's golden panic, proving the golden actually exercises the
new path; full green (`cargo fmt --check && cargo clippy -- -D warnings && cargo
test`).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "tabulate word family: checker signature and IR lowering (spliced quotation call replacing fill's replicated store)", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "nullary-variant fill relaxation via new Slot.variant_idx field, plus the non-nullary-linear-seed rejection diagnostic", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "delete [Type; Count] end to end (parser, checker, IR, diagnostic) and migrate examples/array_ctor.sth to fill", "effort": "S", "difficulty": "standard" },
    { "phase": 4, "focus": "synthesize_array_destructor and array_drop_symbol, wired into emit_drop's IrType::Array arm", "effort": "M", "difficulty": "standard" }
  ]
}
```
