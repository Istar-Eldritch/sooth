# Phase 4 Slice 3: loop-carried aggregate aliasing and the back-edge copy (spec)

Base: `main` @ `728a335`. Design input: [the brief](./phase4-slice3-brief.md), whose seven
recon findings are measured facts and are not reopened here. This spec resolves the brief's six
enumerated decisions (D1–D6) with an explicit pick each, and turns the verified failure modes into
goldens.

This is a **lowering-shape fix to a live silent-miscompile bug**, not a language feature. No new
syntax, no new IR instruction (`Alloc` and `Blit` both exist, recon 6), no new backend surface,
no backend-neutrality decision, and no checker change (the crossing set is already computed and
already guaranteed moved, recon 5). The whole job is to pin down *where the copy goes and in what
order* so that an aggregate carried across a self-tail-call back-edge stops aliasing storage the
next iteration overwrites. It does **not** touch call ABI, non-tail/mutual recursion, references
across the edge, or scalars (all correct as they stand).

The cause is **storage reused across iterations**, not the aggregate-return ABI specifically. A
by-value aggregate return (one QBE stack slot per call site) is the most common instance and the
reason this slice became urgent, but it is one instance, not the mechanism: an aggregate
constructed *inline* each iteration, with no call at all, reuses its entry-hoisted storage and
reproduces the identical miscompile. The spec is framed around storage reuse throughout; the
aggregate return is called out as the common case where it appears.

## The bug, verified live against `728a335`

Every repro below was built and run against the current tree; the "got"/"want" pairs are actual
output, not the brief restated.

- **Struct.** A two-field struct carried across the edge while the producer is called again before
  the old value is read: a 3-step countdown prints `0 2 1 1`; correct is `0 3 2 1`.
- **Array.** The same shape over `[i64 4]` via `4 fill`: prints `0 2 1`; correct is `0 3 2`.
- **Enum.** The same shape over a single-variant enum: prints `0 2 1 1`; correct is `0 3 2 1`. The
  enum case is **not** merely by-construction: it reproduces the identical miscompile live, because
  `Struct`/`Enum`/`Array` share one runtime representation (a pointer to aggregate storage) and one
  return path (recon 2).
- **Destructor (the resource-safety bar, recon 1).** A `Res` wrapping an `i64`, disposed one
  iteration late, prints `1000 1002 1001 1001`; correct is `1000 1003 1002 1001`. Disposal stays
  exactly-once *in form* (the checker's guarantee holds), but the *contents* disposed are wrong:
  one value is disposed twice by content and another never. For a `Res` over a file descriptor or
  a heap pointer this is a double-close and a leak, not a misprinted number.
- **Nested projection (the interior-pointer case).** A loop carrying a `Segment` and a `Vec2`,
  recursing with the segment's `from` field (`s Segment>from`) in the `Vec2` position while
  re-producing the segment, prints `99 0 2 1`; correct is `99 0 3 2` (established the recon-3 way, by
  making the call non-tail). The carried `Vec2` arg is an interior pointer *into* the carried
  `Segment` slot, a distinct pointer from the slot itself, which is why a hazard test keyed on
  pointer identity misses it (D2).
- **Inline constructor (no call at all).** A loop constructing its carried `Vec2` inline each
  iteration and reading the prior value after prints `0 2 1 1`; correct is `0 3 2 1`. No producer
  word: the reused storage is the entry-hoisted constructor slot, the call-free witness that the
  cause is storage reuse, not the return ABI.
- **Two-aggregate swap (recon 7, already correct today).** A loop carrying `a` and `b` and
  recursing with `b a` prints the correct alternation `1 2 1 2 2` and returns the right pair,
  because pointer-phi shuffling is fine when neither aggregate is re-produced in the loop. This is
  the regression guard the plausible fix can break.

## The mechanism (confirmed against source)

A self-tail-recursive word lowers to a **loop**, not a new frame: `lower_word_parts`
(`src/ir.rs:1975`) calls `begin_loop` (`src/ir.rs:2259`) when `has_self_tail_call`
(`src/check.rs:2130`) holds. `begin_loop` seals the entry block with a `Jmp` to a fresh header and
emits one `Instr::Phi` per carried slot, each seeded with the entry arm `(entry, param)`, returning
the phi outputs that the body reads. A tail self-call (`src/ir.rs:2578`) pops the call's arguments
and records `(pred block, args)` in `back_edges` (`src/ir.rs:2114`), sealing the block with a `Jmp`
to the header. `finalize_loop` (`src/ir.rs:2280`) then back-patches each header phi with one arm
per back-edge.

For an aggregate slot the "value" flowing through this is a **pointer into aggregate storage**, not
the bytes. The mechanism of the bug is therefore not the call ABI but **storage reuse across
iterations**: whenever the pointer a slot carries across the back-edge points into storage the next
iteration overwrites, the carried value silently becomes the new contents, with no diagnostic. Two
independent sources of reused storage trigger it, and a correct fix has to cover both:

- **A by-value aggregate return (the common instance, why this slice became urgent).** QBE
  materialises a call's aggregate result into **one stack slot per call site** and never versions it
  (`Instr::Alloc` emits inline, `src/backend/qbe.rs:1029`). A back-edge arg built from that result is
  a pointer into the one slot (`field_value`, `src/ir.rs:3214`, hands back an interior pointer, never
  a copy), so the next call to the same producer overwrites the bytes the slot still points at. Slice
  1 made this ubiquitous by putting every multi-output word on the aggregate-return path.
- **An inline-constructed aggregate, no call at all.** A constructor's storage
  (`alloc_struct`/`alloc_enum`/`alloc_array`, `src/ir.rs:2768`+) is entry-hoisted by `push_alloc`
  while looping (`src/ir.rs:2221`), so it is **one slot reused every iteration**, exactly like a
  per-call-site result slot. A loop that constructs its carried aggregate inline and reads the prior
  iteration's value *after* constructing the new one miscompiles identically, with no producer word
  involved. The `entry_block` doc comment (`src/ir.rs:2116`) already names this exact hazard as a
  "future lowering"; it has arrived. R4's back-edge copy fixes it for free (the arg is a body-local
  slot, not a carried stable slot, so R4 stages it like any non-forwarded arg and the copy is
  correct), so it is a coverage-and-framing gap, not a second defect.

The trigger in both cases is an ordering one, which explains a confusing near-miss: the old value
must be read *after* the new one is produced. Moving the read first makes the same program print
correctly today (the asymmetry the brief noted with `prev drop`). Non-tail and mutual recursion keep
real frames and are unaffected (recon 3).

ROADMAP.md's slice 3 entry attributes the cause solely to the aggregate-return ABI (one QBE slot
per call site). That is the most common instance, not the mechanism; the verified mechanism is
storage reuse across the back-edge, of which the inline constructor is a call-free witness. This
slice does **not** edit ROADMAP.md; phase 3's docs step corrects the causal framing there.

The already-existing loop-body alloc hoisting is the lever. `push_alloc` (`src/ir.rs:2221`) routes
any `Alloc` emitted while looping into the **entry block** instead of the body (`entry_block` is
`Some` inside a loop, `src/ir.rs:2116` documents why: an inline `alloc` in the body bumps the frame
every iteration and never reclaims it, which would blow the constant-stack guarantee this phase
exists to demonstrate). A stable per-slot copy therefore has a ready home: `alloc_struct`/
`alloc_enum`/`alloc_array` (`src/ir.rs:2768`/`:2781`/`:2794`) already go through `push_alloc`, so a
slot allocated during `begin_loop` lands in the entry block, allocated once, reused every iteration.

## Why the obvious fix is dead (recon, restated as constraint)

Copying the call result into a fresh slot **at the call site** fails in both placements. Hoisted to
the entry block it is a single slot again and the bug returns unchanged. Left in the loop body it is
a per-iteration `alloc` (QBE emits `alloc` inline, no hoisting), a stack bump per iteration that
breaks the constant-stack guarantee in the exact feature (`each`/`fold`) meant to demonstrate it. So
the copy must land **on the back-edge**, into a slot **allocated once in the entry block and stable
across iterations**. That turns the fix from a call-site rewrite into phi elimination for
aggregates, which is where the cycle problem (recon 7) comes from.

## Decisions (D1–D6 resolved)

- **D1, which values get a stable slot: the broad rule.** Every aggregate-typed carried slot of a
  self-tail-recursive word gets a stable slot, using the set the loop already carries (recon 5: at
  the back-edge the live aggregate set is exactly the recursive call's arguments, `stack[base..]`,
  the same slice the checker scans). The narrow "only those that can alias a call result produced
  in the loop" rule needs an aliasing analysis that does not exist. **Cost of the broad rule, stated
  so the reader can judge it:** at most two blits per carried aggregate per iteration, a read-phase
  snapshot and a write-phase store (D2 elides a forwarded-unchanged slot to zero). A blit is a fixed-size
  `memcpy` of one aggregate; it does **not** grow the frame (the slot is entry-hoisted, D1's whole
  point), so it costs runtime, never stack. Runtime blit count on the combinator hot path is a
  slice 4–5 concern where inlining changes the calculus; it is not this slice's to optimise. The
  broad rule is checkable by inspection, which matters more here than shaving copies.

- **D2, cycle breaking: read-before-write staging of every non-forwarded arg through a temp.** The
  back-edge is a **parallel assignment** `stable[i] <- arg[i]`. Two facts make a naive write-order
  copy wrong. First, an `arg[i]` may be another carried slot's stable slot (the swap case). Second,
  and the one that kills a pointer-identity hazard test, an `arg[i]` may be an **interior pointer
  into** a carried stable slot without being equal to it: `field_value` (`src/ir.rs:3214`) lowers an
  aggregate field to `Instr::PtrOffset(v, base, offset)` (`field_aggregate_value`, `src/ir.rs:3193`),
  an interior pointer, not a copy. So a back-edge arg formed by projecting a field out of a carried
  aggregate (a nested-aggregate getter `Segment>from`, `src/ir.rs:3374`; a whole-struct destructure
  `Segment>`, `src/ir.rs:3410`; or an enum clause-body payload binding, `src/ir.rs:3698`, all through
  `field_value`) is a *distinct* `Value` from the stable slot it reads. A test that stages only when
  `arg[i]` **is** another stable slot classifies such an arg "fresh", reads it in the write phase,
  and reads it after some other stable slot was already overwritten. The nested-projection repro
  (criterion 7, `99 0 2 1`, correct `99 0 3 2`) is exactly this, and it stays broken under such a
  rule with an outcome that depends on slot write order, which a spec must not leave to chance.

  The fix is a blunt one that is immune to the whole class **by construction**. Partition the carried
  aggregate slots on each back-edge:
  - **forwarded-in-place** (`arg[i]` **is** exactly this slot's own `stable[i]`, i.e. carried
    unchanged): emit **nothing**; the value is already in its stable slot. Elides a
    forwarded-unchanged aggregate, including a forwarded array, to zero blits.
  - **staged** (every other slot): in the **read phase** snapshot it into a per-slot temp,
    `temp[i] <- arg[i]`, before any write; in the **write phase**, `stable[i] <- temp[i]`.

  All read-phase snapshots precede all write-phase stores, so every stable slot an arg reads,
  whether the arg *is* that slot (swap) or points *into* it (a `field_value` projection), is copied
  out before any store lands. This needs **no provenance or aliasing analysis**: it does not matter
  whether an arg aliases a stable slot, because every non-forwarded arg is snapshotted regardless.

  **Why not the narrower pointer-provenance test.** The accurate alternative is a may-alias hazard
  test: walk `arg[i]` back through its defining `PtrOffset` chain (transitively, a nested-nested
  field is possible) to its root and stage only when the root is a carried stable slot. When Value
  identity was believed sufficient that test was an O(carried slots) membership check and clearly
  worth its saved copies; it is **not** sufficient (above), so the accurate version now requires a
  transitive `PtrOffset`-chain walk over a value→defining-instruction index that `FuncBuilder` does
  not have (`src/ir.rs:2082`, no def map). That is the exact aliasing analysis D1 declined to build,
  and getting its may-alias direction wrong reintroduces the silent miscompile. Unconditional staging
  avoids all of it. **Its cost, stated honestly:** every non-forwarded carried aggregate pays two
  blits per iteration (a read-phase snapshot and a write-phase store) where the provenance test would
  pay one for a genuinely fresh arg; it over-stages the fresh-arg case (a producer result that
  aliases nothing) unconditionally, not just the single edge case the previous rule over-staged. That
  extra blit is exactly the per-carried-aggregate copy D1 already accepted and deferred to slices
  4–5, and a blit does not grow the frame (the temp is entry-hoisted), so the cost is runtime, never
  stack. Immunity by inspection is worth more than the copies for a silent-miscompile fix. The
  forwarded-in-place elision is by **exact `Value` identity only**; a hypothetical zero-offset
  self-projection `PtrOffset(stable[i], 0)` typed as slot `i` is byte-identical and could be elided
  too, but it is instead staged (two redundant blits) rather than special-cased, and in any case it
  cannot arise from field projection, since a type cannot contain itself at offset 0. Temps are
  entry-hoisted (constant frame), one per aggregate slot, reused every iteration and back-edge. The
  swap program is criterion 5; the interior-pointer program is criterion 7.

- **D3, the phi: `begin_loop` stops emitting a phi for an aggregate slot.** Since an aggregate slot
  is read from its stable slot on every iteration, its header phi would be degenerate (the same
  stable-slot pointer from every predecessor). `begin_loop` therefore emits **no** `Instr::Phi` for
  an aggregate carried slot; the body reads the stable-slot pointer directly (a value defined in the
  entry block, which dominates the header and body, so no SSA phi is required at all). Scalar slots
  keep their phi unchanged. This is the concrete function that changes (`begin_loop`), and
  `finalize_loop` changes in lockstep to back-patch scalar phis *and* emit the aggregate back-edge
  blits (D2) into each predecessor block. No back-patch of a phi for an aggregate slot exists to
  do, because none was emitted.

- **D4, initial value: an entry-arm init blit.** `begin_loop`, for each aggregate slot, blits the
  incoming param into that slot's stable slot **once in the entry block**, before the loop starts,
  so iteration 1 reads an initialised value. This is its own requirement (R3) because it is easy to
  miss: every existing repro survives iteration 1 (the aliasing miscompile only manifests from
  iteration 2), so a fix that forgot the init would still pass the struct/array/enum/destructor
  countdowns yet corrupt a one-iteration or zero-body-write loop.

- **D5, destructor interaction: the back-edge blit is a move.** The source of a back-edge blit is
  dead after it, guaranteed by `check_linear_across_back_edge` (`src/check.rs:4062`, R15): the only
  live aggregates at the edge are the call's arguments, and each is moved into the call. So the blit
  creates **no second live copy**, and the exactly-once checker's accounting is untouched by
  lowering. What the fix changes is *which bytes* a later disposal sees: with a stable slot, the
  carried param (e.g. `prev`) reads the stable slot, which still holds the correctly-carried value,
  so `prev drop` disposes the right resource *before* the back-edge blit overwrites the slot for the
  next iteration. This is the acceptance bar (recon 1) and is criterion 4, a run that prints the
  correct disposal sequence, not an abstract argument.

- **D6, non-aggregate carried values do not change.** A scalar is copied out by value and is
  already correct; its slot keeps its `Instr::Phi` exactly as today (`begin_loop`/`finalize_loop`
  scalar path unchanged). References across the back-edge are out of scope: a reference whose owned
  root is a frame local is already a hard error (`check_reference_across_back_edge`,
  `src/check.rs:4041`, recon 4), and a reference into an ancestor frame is legal and unaffected.
  This is written as R6 so the implementation does not generalise the fix into a rewrite of loop
  lowering.

## Requirements by stage

Requirement IDs `Rn`. "Golden" means source-in → expected-output, runnable, **never** an
IL-string / emitted-IL assertion (CLAUDE.md). This is a miscompile fix: there are **no new
diagnostics** and therefore **no `Xn`** (the reference form was already rejected, recon 4; the value
form was a silent miscompile now made correct). All changes are in one stage, lowering
(`src/ir.rs`), plus the goldens in `tests/`.

**R1: A stable frame slot per carried aggregate (D1).** In `begin_loop`, for each carried slot whose
`value_type` is `Struct`/`Enum`/`Array`, allocate one stable frame slot via the matching
`alloc_struct`/`alloc_enum`/`alloc_array` (which route through `push_alloc` and so land in the entry
block, allocated once and reused every iteration). The entry_value returned for that slot is the
stable-slot pointer, which the body reads. Scalar slots are unchanged. The set is the loop's own
carried slots (`params_values` / `stack[base..]`), i.e. recon 5's set; no analysis beyond a
per-slot `value_type` classification.

**R2: `begin_loop` emits no phi for an aggregate slot; the body reads the stable slot (D3).** The
header carries a phi only for scalar carried slots. An aggregate slot contributes no `Instr::Phi`;
its entry_value is the R1 stable-slot pointer. The per-slot metadata that `finalize_loop` consumes
must record, per carried slot, whether it is a scalar (carrying its phi `Value`) or an aggregate
(carrying its stable-slot `Value`, size/align, and a per-slot temp `Value` for D2 staging), so
finalize can no longer index phis positionally against a flat `header_phis` list.

**R3: Entry-arm init blit (D4).** In `begin_loop`, for each aggregate slot, emit `Blit(param,
stable, size)` into the entry block (via `push_alloc`, so it lands after the slot's `Alloc` and
before the entry block's terminating `Jmp`), copying the incoming param into the stable slot once
before the loop runs. A zero-size aggregate emits no blit (the existing `size > 0` guard).

**R4: Back-edge move blit with unconditional read-before-write staging (D2).** `finalize_loop`, for
each back-edge `(pred, args)`, and for each aggregate carried slot `i`:

1. **forwarded-in-place** (`args[i]` == this slot's `stable[i]`): emit nothing.
2. **staged** (otherwise): in the read phase emit `Blit(args[i], temp[i], size)`; in the write phase
   emit `Blit(temp[i], stable[i], size)`.

   All read-phase blits precede all write-phase blits. This holds whether `args[i]` *is* another
   carried stable slot (swap) or is an interior pointer *into* one (a `field_value` projection): the
   snapshot copies the bytes out before any store overwrites them, with no aliasing analysis. The
   blits are appended to the **predecessor block's** instrs (the block is already sealed with its
   `Jmp` to the header; appending to `block.instrs` lands the blits before the stored terminator).
   Scalar slots keep the existing phi-arm back-patch. Temps are entry-hoisted (R1-style
   `push_alloc`), one per aggregate slot, reused across iterations and back-edges (each back-edge
   fully completes its read-then-write before the next iteration).

**R4a: Correct the `entry_block` doc comment (`src/ir.rs:2116`).** Its closing sentence asserts that
same-site alloc hoisting is "safe only because a same-site slot is read ... before the next iteration
overwrites it", and names a lowering that constructs into a same-site slot before reading the prior
value as a hypothetical future hazard. Criterion 8 shows that lowering already exists, so the comment
currently asserts an invariant the compiler does not hold. After R4 the safety argument is different:
a carried aggregate is copied into its stable slot on the back-edge, so hoisting no longer depends on
body read order at all. Rewrite that sentence to state the R4 reason. This is in phase 2, not the
docs phase, because it is a comment on the function R1-R4 change and it would otherwise be left
contradicting the code around it (CLAUDE.md: a comment carries the WHY, so a false WHY is worse than
none).

**R5: The blit is a move; disposal disposes the right contents (D5).** No requirement code beyond
R3/R4: the move property is a consequence of R15's crossing-set guarantee (source dead after the
edge) and is asserted behaviourally by criterion 4, not by an added check. The fix adds and removes
no drop glue; `emit_drop` and destructor synthesis are untouched.

**R6: Scalars and references unchanged (D6).** A scalar carried slot keeps its `Instr::Phi` and its
back-edge phi-arm; nothing in the scalar path changes. No reference can reach this path as a carried
aggregate (a frame-local reference across the edge is already a hard error, recon 4; a `Type::Ref`
never becomes a carried aggregate slot). Regression: the full existing suite passes unmodified,
including `examples/countdown.sth` and every scalar-carried constant-stack golden in `tests/phase0.rs`.

**R7: Uniform over `Struct`/`Enum`/`Array` (recon 2).** R1–R4 dispatch on the three aggregate
`value_type` arms identically (each has its `alloc_*` and its layout `size`); the fix is uniform
over the three or it is wrong for two of them. Pinned by criteria 1, 2, 3 (one per kind).

**R8: No new IR, no backend change, no checker change (recon 5, 6).** Only `Alloc`/`Blit` (both
extant) are emitted; `src/backend/qbe.rs` is untouched; `src/check.rs` is untouched. The stable/temp
slots are memory (QBE `alloc`), not SSA values needing a phi, so a pointer defined in the
entry block is valid everywhere the header/body dominate: this is exactly why a stable *slot*
sidesteps the aggregate-phi problem.

**R9: Constant stack preserved (D1 corollary).** Every `Alloc` the fix introduces (stable slots and
temps) is entry-hoisted via `push_alloc`, so the frame grows by a fixed number of slots independent
of iteration count; no per-iteration `alloc` is added. Pinned by criterion 6 (a fixed-count
aggregate-carrying loop run to 1,000,000 iterations under a 1 MB stack, `ulimit -s`, reusing the
slice-6 precedent).

## Success criteria

Every criterion is a runnable golden (source-in → expected-output), named
`thing_condition_expected`. New goldens live in `tests/phase4_generics.rs`, which already exists,
already targets the Phase 4 aggregate-return ABI these programs exercise, and already has the
`run_src(name, src, trace)` helper (`tests/phase4_generics.rs:12`); the destructor case mirrors the
resource-ordering goldens of `tests/phase3_resources.rs` but is kept here so the loop-carried-
aggregate goldens that change together stay together (CLAUDE.md growth structure). The constant-
stack criterion needs a signal-aware runner (a stack overflow is a `SIGSEGV`, which `run_src`'s
`.code().expect(...)` would panic on): add a small `ulimit -s`-bounded helper local to this test
file, copying the shape of `run_stack_bounded_golden` (`tests/phase0.rs:2723`), since integration
test files do not share helpers.

| # | criterion | golden name | maps |
|---|---|---|---|
| 1 | the struct repro prints `0 3 2 1` (was `0 2 1 1`) | `struct_carried_across_back_edge_is_not_aliased_expected` | R1–R4, R7 |
| 2 | the array repro over `[i64 4]` via `4 fill` prints `0 3 2` (was `0 2 1`) | `array_carried_across_back_edge_is_not_aliased_expected` | R1–R4, R7 |
| 3 | the enum repro prints `0 3 2 1` (was `0 2 1 1`) | `enum_carried_across_back_edge_is_not_aliased_expected` | R1–R4, R7 |
| 4 | the destructor repro prints `1000 1003 1002 1001` (was `1000 1002 1001 1001`): disposal is exactly-once **and** disposes the right contents (the resource-safety witness, recon 1) | `destructor_carried_across_back_edge_disposes_right_contents_expected` | R4, R5 |
| 5 | the two-aggregate swap program still prints `1 2 1 2 2` and returns the right pair (the D2 regression guard) | `two_aggregates_swapped_across_back_edge_stay_correct_expected` | R4 (D2) |
| 6 | a fixed-count aggregate-carrying loop that re-produces its carried aggregate each iteration runs to 1,000,000 iterations under a 1 MB stack (`ulimit -s`) and exits 0: the fix introduced no per-iteration stack bump | `aggregate_carried_loop_runs_in_constant_stack_expected` | R9 |
| 7 | the nested-projection repro prints `99 0 3 2` (was `99 0 2 1`): a back-edge arg that is an interior pointer *into* a carried stable slot is snapshotted before the slot is overwritten (the D2 staging guard, and the one case the superseded pointer-identity rule got wrong) | `nested_projection_carried_across_back_edge_is_not_aliased_expected` | R4 (D2) |
| 8 | the inline-constructor repro prints `0 3 2 1` (was `0 2 1 1`): a carried aggregate built inline each iteration, with no producer call, is not aliased across the edge (the storage-reuse witness that the cause is not the return ABI) | `inline_constructed_aggregate_carried_across_back_edge_is_not_aliased_expected` | R1–R4 |

**Enum golden (criterion 3), decision and reason.** It is **not** redundant and is included.
Recon 2 argues the enum follows by construction, but it was verified to reproduce the identical live
miscompile (`0 2 1 1`), and the fix's central claim is "uniform over the three or wrong for two."
One extra golden converts a by-construction claim into a checked fact for the price of one test,
which is proportionate to a silent-miscompile fix and matches the house rule that "verified" means
run.

**Surface-form coverage (criterion 7), decision and reason.** The interior-pointer hazard has three
surface forms, all funnelling through one producer, `field_value` → `field_aggregate_value`'s
`PtrOffset` (`src/ir.rs:3214`/`:3193`): a nested-aggregate getter (`Segment>from`, `src/ir.rs:3374`),
a whole-struct destructure (`Segment>`, `src/ir.rs:3410`), and an enum clause-body payload binding
(`src/ir.rs:3698`). Unlike the aggregate-*kind* axis (Struct/Enum/Array have three distinct
`alloc_*`/layout arms, so each earns its own golden, criteria 1–3), these three are one surface
syntax over one interior-pointer instruction and one back-edge staging path; R4's staging does not
depend on how the pointer was produced. So the getter golden (criterion 7) is the interior-pointer
witness **by test**, and the destructure and enum-payload forms are covered **by construction** (same
`field_value` producer); goldens for them would re-test the same instruction, unlike the enum-*kind*
golden which converts a distinct-code-path claim into a checked fact.

**Confirmed non-producers, so the fix stays bounded.** A read *through a reference* (`@`) does not
yield an alias: `lower_access_word` (`src/ir.rs:2717`) copies an aggregate referent via
`alloc_aggregate` + `Instr::Blit` into fresh storage. Array element access is reference-mediated
(`&>` then `@`, `examples/stack.sth`), so `Instr::ElemAddr` (`src/ir.rs:2840`) never produces an
aggregate *value* aliasing a carried slot. The interior-pointer hazard is therefore confined to the
three `field_value` forms above.

## Delivery plan

Two substantive phases plus a docs phase. Each phase ends **green**
(`cargo fmt --check && cargo clippy -- -D warnings && cargo test`). The swap-regression guard
(criterion 5) lands in phase 1, **before** the lowering change that could break it, alongside the
`ulimit -s` helper; it passes on the current tree because the swap program is already correct today.
The interior-pointer and inline-constructor witnesses (criteria 7, 8) are **red** on the current
tree and can only land with the fix (phase 2): a regression guard must be green before the change so
a break is visible, while a red-to-green fix-witness cannot exist as a guard until the code it
asserts is written. Both sides pin the D2 hazard, criterion 5 from correct-today and criterion 7
from broken-today.

1. **Regression guards (green on the current tree).** Add criterion 5 (the swap golden, passing on
   `728a335`) and the file-local `ulimit -s`-bounded signal-aware runner used by criterion 6. No
   lowering change. This exists so the D2 hazard is pinned before the fix can regress it.
2. **The lowering fix + correctness goldens.** Implement R1–R9 in `begin_loop`/`finalize_loop` and
   the `FuncBuilder` carried-slot metadata: entry-hoisted stable slot per aggregate (R1), no
   aggregate phi (R2), entry-arm init blit (R3), back-edge unconditional read-before-write staging
   with forwarded-in-place elision (R4), scalars/references untouched (R6). Land criteria 1–4
   (struct, array, enum, destructor), criterion 6 (constant stack), and criteria 7–8 (the
   nested-projection interior-pointer witness `99 0 3 2` and the inline-constructor storage-reuse
   witness `0 3 2 1`), both red on the current tree and green only with the fix. Ends green with the
   miscompile gone.
3. **Docs.** Remove the aggregate-return-aliasing known-issue text from `ROADMAP.md`'s Phase 4 slice
   3 entry (and note it is fixed), and correct its causal framing: the cause is storage reuse across
   the back-edge, of which the aggregate-return ABI is the most common instance but not the
   mechanism (an inline-constructed carried aggregate reproduces it with no call). Record the D1–D6
   picks in `DESIGN.md` if it carries a deferred-decisions log. No code change.

## Out of scope

Quotations and combinators (slices 4–5). Non-tail and mutual recursion (real frames, unaffected,
recon 3). References across the back-edge (already rejected, recon 4). Any change to the call ABI,
which is correct. Any aliasing analysis to narrow D1's set (the broad rule is deliberate). Full SCC
cycle detection and any pointer-provenance may-alias analysis on back-edge args (D2's unconditional
staging is the deliberate cheaper-and-simpler pick, immune by construction, and avoids the def-use
index `FuncBuilder` lacks). The runtime
blit-count optimisation on the combinator hot path (slice 4–5, where inlining changes the calculus).
Any new diagnostic (this is a miscompile fix, not a new rejection).

## Key risks

- **The parallel-copy / interior-pointer hazard (D2/R4).** A back-edge arg can read a carried stable
  slot in two ways: by *being* another slot (the swap) or by being an interior pointer *into* one (a
  `field_value` projection). Emitting the blits in write order, or staging only args that *equal* a
  stable slot, corrupts both: the swap regresses (criterion 5) and the projection reads an
  already-overwritten slot (criterion 7). Mitigation: unconditional read-before-write staging (every
  non-forwarded arg snapshotted into a temp before any store), immune to both by construction with
  no aliasing analysis. Pinned by criterion 5 (green before the fix, phase 1) and criterion 7 (red
  before, green after, phase 2).
- **Forgotten init (D4/R3).** Every countdown repro survives iteration 1, so a fix missing the
  entry-arm blit would still pass criteria 1–4 yet corrupt a zero-body-write or one-iteration loop.
  Mitigation: R3 is a distinct requirement; the init blit is emitted unconditionally per aggregate
  slot in `begin_loop`.
- **Destructor contents (D5/R5).** The blit must be a move, or a second live copy would be invisible
  to the exactly-once checker and could double-dispose. Mitigation: the crossing set is guaranteed
  moved by R15 (`check_linear_across_back_edge`); criterion 4 is the live witness, not an argument.
- **Slot-metadata indexing (R2).** Dropping aggregate phis breaks `finalize_loop`'s positional
  `header_phis` indexing. Mitigation: replace the flat phi list with a per-carried-slot record
  (scalar-phi vs aggregate-stable-slot+temp), so finalize dispatches per slot kind rather than by
  phi position.
- **A per-iteration `alloc` sneaking in (R9).** If a stable slot or temp is allocated in the body
  rather than hoisted, the frame grows per iteration and the constant-stack guarantee breaks.
  Mitigation: all such `Alloc`s go through `push_alloc`, which hoists to the entry block while
  looping; criterion 6 runs 1,000,000 iterations under a 1 MB stack and fails loudly (SIGSEGV) if
  the hoist is missed.

## Current-state anchors (confirmed against `728a335`)

- `IrType::Struct(StructId)` / `Enum(EnumId)` / `Array(ArrayId)`: `src/ir.rs:99`/`:106`/`:112`
  (`enum IrType` at `:76`). All three are a pointer to aggregate storage at runtime. → R7.
- `Instr::Alloc(Value, size, align)`: `src/ir.rs:929`; `Instr::Blit(src, dst, size)`:
  `src/ir.rs:933` (first operand is the source, per `src/backend/qbe.rs:1038`). → R1, R3, R4, R8.
- `field_value` (hands back an interior pointer into a slot, not a copy): `src/ir.rs:3214`, via
  `field_aggregate_value`'s `Instr::PtrOffset`: `src/ir.rs:3193`. → the bug's mechanism, D2/R4.
- The three surface forms that put a `field_value` interior pointer on a back-edge: nested-aggregate
  getter `StructWord::Get` (`src/ir.rs:3374`), whole-struct destructure `StructWord::Destructure`
  (`src/ir.rs:3410`), enum clause-body payload binding (`src/ir.rs:3698`). → D2/R4, criterion 7.
- `lower_access_word` (`@` copies an aggregate referent via `alloc_aggregate` + `Blit`, so a
  read-through-reference is never an alias): `src/ir.rs:2717`; array element access is
  reference-mediated, `Instr::ElemAddr` at `src/ir.rs:2840`. → confirmed non-producers.
- `FuncBuilder` has no value→defining-instruction map: `src/ir.rs:2082`. → why a provenance
  may-alias test would be new machinery, and D2 declines it.
- The `entry_block` doc comment naming the inline-aggregate aliasing hazard as a "future lowering":
  `src/ir.rs:2116`. → the inline-constructor witness, criterion 8.
- `begin_loop` (entry block binds params, jumps to header, one seeded `Instr::Phi` per slot):
  `src/ir.rs:2259`. → R1, R2, R3.
- `finalize_loop` (back-patches each header phi with one arm per back-edge): `src/ir.rs:2280`.
  → R2, R4.
- `back_edges: Vec<(BlockId, Vec<Value>)>`: `src/ir.rs:2114`; the tail self-call that pushes
  `(cur_id, args)` and seals with `Jmp(header)`: `src/ir.rs:2578`+. → R4.
- `push_alloc` (hoists an `Alloc` to the entry block while looping; `entry_block` doc comment
  explaining the constant-stack reason): `src/ir.rs:2221` (doc at `:2116`). → R1, R3, R4, R9.
- `alloc_struct`/`alloc_enum`/`alloc_array` (each `push_alloc`s an `Alloc` and returns the value):
  `src/ir.rs:2768`/`:2781`/`:2794`. → R1.
- `has_self_tail_call`: `src/check.rs:2130`; called from `lower_word`: `src/ir.rs:1926`. → the
  loop-vs-frame gate.
- `check_reference_across_back_edge` (frame-local reference across the edge is a hard error, recon
  4): `src/check.rs:4041`. → D6/R6.
- `check_linear_across_back_edge` (R15: the crossing set is exactly the moved call args, nothing
  else live, recon 5): `src/check.rs:4062`. → D5/R5.
- `Instr::Alloc` lowering (inline, no hoisting: QBE `alloc4`/`alloc8`/`alloc16`): `src/backend/qbe.rs:1029`.
  → "obvious fix is dead" and R9.
- Constant-stack test precedent: `run_stack_bounded_golden` under `ulimit -s 1024`:
  `tests/phase0.rs:2723`; a 1,000,000-iteration golden shape: `tests/phase0.rs:1464`. → criterion 6.
- The `run_src` golden helper: `tests/phase4_generics.rs:12`. → criteria 1–5, 7, 8.

## Non-functional

- **Green** unchanged: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- **No new `Instr`/`Terminator`**; the fix reuses `Alloc`/`Blit` and the existing loop shape (R8).
- **No checker change**; the crossing set and its move guarantee already exist (R5, R6).
- **No backend change**; `src/backend/qbe.rs` is untouched (R8).
- **Backend stays QBE**; `Ptr` stays opaque (a stable slot is an opaque aggregate pointer, never a
  `u64`). No LLVM, no native backend, no WASM assumption broken.
- **`core` stays `no_std`**; no JIT, no comptime interpreter, no inliner.
- **Constant stack** is preserved: every introduced `Alloc` is entry-hoisted (R9).

## Phases JSON

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Regression guards, green on the current tree (728a335). Add the two-aggregate swap golden (criterion 5: a loop carrying two aggregates a and b recursing with b a prints 1 2 1 2 2 and returns the right pair), which passes today and pins the D2 read-before-write hazard before the fix can regress it. Add a file-local ulimit -s-bounded, signal-aware runner to tests/phase4_generics.rs copying the shape of run_stack_bounded_golden (tests/phase0.rs:2723), used by phase 2's constant-stack golden, since integration test files do not share helpers. No lowering change. Exit: criterion 5 green.",
      "effort": "low",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "The lowering fix in src/ir.rs (begin_loop at :2259, finalize_loop at :2280, and FuncBuilder carried-slot metadata replacing the flat header_phis list). R1: one entry-hoisted stable frame slot per carried aggregate slot via alloc_struct/alloc_enum/alloc_array through push_alloc. R2: begin_loop emits no Instr::Phi for an aggregate slot; the body reads the stable-slot pointer (entry block dominates, so no phi needed); scalar phis unchanged. R3: entry-arm init blit param -> stable in the entry block (D4). R4: finalize_loop emits back-edge move blits into each predecessor block with unconditional read-before-write staging: elide a forwarded-in-place arg (arg is exactly its own stable slot), and stage every other arg through a per-slot entry-hoisted temp (read-phase snapshot temp <- arg, write-phase store stable <- temp, all reads before all writes), which is immune by construction to a back-edge arg that is an interior pointer into a carried stable slot (a field_value PtrOffset projection, a distinct Value from the slot) as well as to the two-aggregate swap, needing no aliasing analysis (D2). R4a: rewrite the entry_block doc comment's closing safety sentence (src/ir.rs:2116), which asserts same-site hoisting is safe only because a same-site slot is read before the next iteration overwrites it and names the inline-constructor hazard as hypothetical: criterion 8 shows it already exists, and after R4 the safety reason is the back-edge copy, not body read order. R6: scalars and references untouched. R7: uniform over Struct/Enum/Array. R8/R9: no new IR, no backend or checker change, every introduced Alloc entry-hoisted. Land goldens: struct 0 3 2 1 (criterion 1), array 0 3 2 (criterion 2), enum 0 3 2 1 (criterion 3), destructor 1000 1003 1002 1001 (criterion 4, the resource-safety bar), the nested-projection interior-pointer witness 99 0 3 2 (criterion 7), the inline-constructor storage-reuse witness 0 3 2 1 (criterion 8), and the 1,000,000-iteration aggregate-carrying loop under a 1 MB stack (criterion 6); criteria 7 and 8 are red on the current tree and go green only here, unlike the phase-1 swap regression guard which is green before the fix. Exit: criteria 1, 2, 3, 4, 6, 7, 8 green with the miscompile gone.",
      "effort": "high",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Docs only. Remove the aggregate-return-aliasing known-issue text from ROADMAP.md's Phase 4 slice 3 entry, note it is fixed, and correct its causal framing: the cause is storage reuse across the back-edge, of which the aggregate-return ABI is the most common instance but not the mechanism (an inline-constructed carried aggregate reproduces it with no call). Record the D1-D6 picks in DESIGN.md if it carries a deferred-decisions log. No code change. Exit: green, docs updated.",
      "effort": "low",
      "difficulty": "standard"
    }
  ]
}
```
