# P7.S3h: an escaping closure may capture a linear value (closure env disposal)

**Shipped.** Delivered as three phases on `impl/slice3h_spec-2608241345`; this document records
the design as built, not the delivery plan. Anchors are indicative, not contracts.

## Problem

Capture into an escaping (materialized, non-spliced) closure was gated by
`check_capture_admission`, which classified every captured name by its `Binding.ty` alone:
`Struct | Enum | Array | OwnedCell` → always `FrameRooted`, rejected at an escaping boundary by
`past_owning_frame_error`. Two distinct defects sat behind that one arm:

- The arm was **not gated on linearity or `is_copy`**, so a `bool` local (a payload-free,
  structurally-`Copy` enum since S3i) was rejected purely for being spelled as an enum.
- A genuinely linear capture had **no storage that outlives the frame**. Probes established
  that a quotation body consuming a captured linear value already worked (that is how a
  spliced body behaves) and that the must-consume-on-every-path discipline already existed,
  including for anonymous stack values. Exactly one gate blocked
  `: mk ( Spy -- [ -- ] ) | s | [ s drop ] ;`, and it was a storage-lifetime error, not a
  disposal or linearity one.

## Design

### The obligation is in the type; the storage is not

A consumer discharges the obligation and cannot see the captures: `Type::Quotation` is keyed by
declared effect only, which is what lets `times`/`each`/`map`/`fold`/`filter`/`while` and
`dispatch` be writable at all. Putting the capture set in the type would make every one of those
signatures unwritable; putting the *storage* in the type would let a combinator accept only
closures whose storage it happens to name.

So: **a marker in the type, naming the obligation and nothing else.**
`Type::OwningQuotation(&'static QuotEffect)` (`src/ast.rs:2278`), built by
`owning_quotation_type` (`:2329`), mirroring `Type::InlineQuotation`: same payload, split purely
to carry a capability difference, structural `PartialEq` giving `OwningQuotation(e) != Quotation(e)`
for free. Its `name_static` is prefixed `owning `, so no two of the three quotation variants ever
share a `&'static QuotEffect`. `is_copy` is `false` for it (`check/builtins.rs:252`); `is_linear`,
move tracking, the `dup` gate, the forgotten-value error and the consumed-on-every-path check are
all inherited with no parallel machinery.

### The body is the disposer, so the closure must be called

An owning closure is a word with its arguments already supplied. A word taking a linear argument
needs no disposal metadata: it is always called. A closure differs in exactly one way, that it
might never be called, so the design closes that gap by requiring it:

- The owning closure value is linear, so it must be consumed on every path.
- **`call` is its consuming use, with no checker change**: `call` already pops its receiver and
  never re-pushes it, so the pop discharges the obligation.
- **`drop` on an owning closure is a located rejection** on both the concrete
  (`check.rs:3300`) and generic (`check/poly.rs:1254`) paths: no destructor can run a closure body.
- **The body must consume each linear capture.** This did need new checker work, contrary to the
  plan's "no checker change": `owning_capture_not_consumed_error` (`check/captures.rs:477`)
  rejects a body that captures a linear value and only borrows it, per branch-join arm as well.
- **The body frees its own env**, being the only code that statically knows the layout, and now
  guaranteed to run exactly once.

What this deletes: no `drop` function pointer in the value, no third slot, no widened
`QuotLayout`, no new backend aggregate, no `emit_drop` arm, no per-literal env struct minting, no
module symbol map, no check/lower quotation-numbering invariant.

### Containment: an owning closure is never reachable through an aggregate

If a struct could hold an `owning` field, the struct would be non-`Copy` and so `drop`ping it a
legal consumption, but the container's disposal cannot dispose the closure: `emit_drop`'s
`_ => {}` silently swallows a quotation, and `field_is_linear` / `layout_field_is_linear`
(`src/ir/layout.rs`) also fall to `_ => false`, so `StructLayout::is_linear` is false and
`synthesize_aggregate_destructors` synthesizes nothing. The container's `drop` is a complete
no-op: the capture's own `drop` never runs, the env block is never freed, and the obligation is
discharged without the body running.

**Rule: an owning quotation is rejected in every aggregate position** (struct field, enum variant
field, array element, slice element, owned-cell payload, `extern:` boundary, reference referent).

**This cost no new gate, deliberately.** `audit_quotation_type_registries` rejects through
`reject_quotation_type_position`, which dispatches on the `is_quotation_type` accessor, and its
legal-position carve-outs match `Type::Quotation(_)` structurally. So once `is_quotation_type`
returned `Some` for the new variant, an owning quotation in any of those positions fell straight
past the carve-out into the existing rejection, on the native `check()` path and at the REPL both.
No `is_copy`-based struct-field gate was added: one would reject the ordinary linear struct fields
the language supports.

**One honest exception: the synthesized multi-output bundle.** The rule holds at every *declared*
type position, because that is where the audit reads. A word with two or more outputs gets a
synthesized return-bundle struct, interned by `intern_output_bundles` *after* the type-level
audits run, so an `owning` output does reach that struct as a field. It stays sound: the bundle is
a destructor-free transient ABI carrier, `is_bundle`-flagged with no synthesized `drop`, unpacked
at the call site the instant the word returns. The owning value flows straight back out as a
linear stack value, so its call-once obligation is never handed to a container that could no-op
its disposal, and the bundle is never itself disposed as a container.

- **Array and slice elements reject, but not symmetrically.** Both are caught by the non-`Copy`
  element gates (`check/declarations.rs`), but only the array position is *additionally* covered
  by the audit: the audit never walks `module.slices`, so `check_slice_element_gate` is the sole
  gate holding the slice position. Measured at phase 2: stub the array gate and the audit still
  rejects; stub the slice gate and `Slice[owning [ -- ]]` reaches `ir_type_of` and ICEs. Any later
  slice lifting an element gate must respect that asymmetry.
- **`field_is_linear` and `layout_field_is_linear` are untouched on purpose.** With the rule in
  force neither can ever see an owning quotation; "fixing" them would quietly reopen the hole.
- **Containment is a check-time gate on declared type positions only.** The runtime owned-cell
  wrap `^` applied to a quotation *value* is not check-gated on every route: `^ mk`, where `mk`
  returns a quotation, slips past the operand guard and is stopped only by a backend
  `unreachable!` in `src/backend/qbe.rs` (around `:531`). This is pre-existing and not specific to
  this slice: a plain, non-owning quotation value ICEs identically there. It is called out as a
  known boundary, not a defect introduced here: if that backend `unreachable!` is ever made to
  emit the blit naively, an owning closure could land in a cell whose `drop` is a no-op for a
  quotation payload, so a check-time gate on `^` over a quotation value is needed there first.

Cost: an owning closure cannot be stored in a data structure. It can be created, passed, returned
and called. Lifting that needs a disposer the synthesized glue can invoke, which is **P7.S3v**,
after **P7.S3u** (trait objects) supplies the erased-owner mechanism.

### Surface syntax: `owning [ … ]`, in type positions only

`^[ … ]` was rejected: `^` means heap cell, which answers the storage question the type must stay
silent on, and `^[` already lexes as two tokens for owned-cell-of-array.

`owning` (`OWNING_QUOTATION_KEYWORD`, `ast.rs:2347`) needed no lexer change but did need a parser
prefix branch at **every** type-position entry (`parse_type_expr`, `parse_slot`, `parse_poly_slot`
and the recursion for fields, elements, referents and effect lists), because type dispatch is
first-token only and never looks ahead. Three located diagnostics came with it:

- `owning` as a `type:` or variant name is a reserved-name rejection (`parser.rs:229`), mirroring
  the `Slice` and `^`-name reservations, since the interception sits ahead of every user registry.
- `owning` not followed by a quotation effect blames the prefix, not the next token
  (`owning_without_effect_error`).
- An `owning` effect carrying a type variable is rejected (`polymorphic_owning_quotation_error`):
  `PolyType::Quotation` has nowhere to record the flavour, so folding one would silently produce a
  plain quotation.

**Owningness is inferred at the literal and declared in the type.** There is no term-level syntax:
a literal stays `[ … ]` and its owningness is derived at the materialization boundary from the
declared slot (`captures.rs:436`, `terms.rs:1458`). `owning` in a term position is an unknown
word. A literal capturing a linear value at a plain boundary gets a remedy line naming `owning`;
a `Copy`-aggregate or borrow capture keeps the bare message, because no disposal obligation
addresses a pointer into dead frame storage.

### Env storage: heap, freed at body entry

The env is a raw `sooth_alloc` block laid out per literal by `owning_env_slots`
(`ir/func_builder/mod.rs:752`), one slot per capture, computed identically on the build side
(`build_owning_env`) and the read side (`bind_owning_env`) from the shared `EnvCapture` list. Not
`intern_owned_cell_type` as planned: a cell holds one payload and so cannot hold N captures. A
capture-free owning literal allocates nothing and keeps the null-env shape.

The captured linear value is *moved* into the block, satisfying `Scope::leave`'s unconsumed-local
check, and the block outlives the return. **The body's prologue copies every capture out into its
own frame and frees the block at entry**, not before returning: once each capture is frame-local,
nothing the body computes or returns can alias the freed storage, whereas an exit free would need
a per-path aliasing proof this slice has no machinery for, and an `owning [ -- Spy ]` body (hand
the capture back rather than dispose it) would break immediately under one. Inline and static envs
(the allocation-free `fixed`-layer case) remain follow-on work; the type is silent on storage, so
they can land without touching a signature.

## What shipped, by phase

**Phase 1** (`a7da3fd`) — the classifier fix. The aggregate arm now splits on **scalar
representation**: `capture_is_scalar_represented(ty, enums)` (`captures.rs:167`) is true exactly
for a payload-free enum, computed from the declaration (every variant field-free) rather than from
`ir::layout`, following slice 10c's `tag` domain check, so the answer is available before any
lowering runs. That admits `bool` and nothing else, and keeps rejecting `[i64 4]` and every `Copy`
struct: the env slot is one word and `build_env`'s single-capture arm stores the value inline, but
an aggregate's value is a pointer into frame storage (probed: an in-frame closure over a mutated
`[i64 4]` printed the mutated value, so the env aliases the frame), which at an escaping boundary
is a use-after-return.

*Delta from plan:* the planned `is_copy` conjunct was dropped as vacuous, since `is_copy`'s enum
arm folds over the variant fields and so calls every payload-free enum `Copy` by construction. The
predicate therefore needs `enums` only, so the planned `Ctx::arrays()` accessor and the widened
`check_capture_admission` parameter list were not needed. No existing test flipped, as planned.

**Phase 2** (`8b06d22`, `7a985be`, `a113622`) — the type, the syntax, the containment rule, and a
check-side guard covering both materialization *and* every lowerable declaration position, because
a declared owning parameter reaches `ir_type_of` through signature lowering without ever crossing
a materialization boundary.

**Phase 3** (`07f1243`, `9ce48ae`, `1b3971c`, `58affae`) — representation and the call-once
lifecycle. `IrType::OwningQuotation(QuotSigId)` shares the plain two-word `(code, env)` aggregate
under the same `:Q{n}` symbol; the distinct `IrType` carries only the env *storage* decision into
lowering, not a distinct shape. `EnvPlan::OwningEnv` drives the prologue. `past_owning_frame_error`
is skipped for a linear capture at an owning boundary (`owning && linear`), and the multi-capture
deferral plus its transitive twin are lifted for the owning path only
(`bundle = names.len() >= 2 && !owning`), the in-frame stack-bundle path unchanged.

*Delta from plan:* phase 2's declaration guard was **narrowed, not lifted**.
`reject_owning_quotation_declarations` (`check/audits.rs:476`) survives and rejects two
declarations, each for its own reason:

- an `inline` (spliced) word declaring an owning parameter: a spliced quotation parameter is never
  a runtime value, so the splice route compares only the inline-vs-plain axis and a plain literal
  would silently satisfy an `owning` slot, with no heap env ever built;
- a *generic* signature declaring one: a polymorphic call site materializes its quotation
  arguments off `CallInst::quot_inputs`, which records the effect and not the flavour, so the
  parameter would be built with a plain closure's frame env.

It runs *after* `check_types`, so `dup`ping or forgetting an owning binding reports its own
inherited error rather than being masked.

## Invariants

- Plain `Type::Quotation` stays `Copy`: no new obligation, no `dup` ban, no IL or golden churn for
  any program that exists today, and `type :Q{n} = { l, l }` stays byte-identical.
- `is_linear` remains the single source of every exactly-once obligation.
- `OwningQuotation(e) != Quotation(e)` structurally, so every boundary and `if`-join separates
  them by type inequality before lowering.
- The type names the obligation only. **Storage never appears in the type.**
- **An owning quotation is never a field, element, payload or referent**, so no synthesized
  destructor and no `emit_drop` path can reach one. This is what makes "the body is the sole
  disposer" true rather than aspirational.
- A capture is snapshotted into the one-word plain env only when it is **scalar-represented**;
  aggregate-backed values are pointers and are never snapshotted at an escaping boundary.
- `call` is a consuming use and needs no checker change; `drop` on an owning closure is rejected.
- Owningness is inferred at literals and declared in types. No term-level `owning` syntax.

## Out of scope

- The case-2 aggregate-parameter/global narrowing in `classify_capture`.
- Capturing an already-quotation-typed name by value (`captured_quotation_name_deferred_error`),
  which the `is_quotation_type` → `Some` answer necessarily also defers for owning-typed names.
- **Storing an owning closure in an aggregate, and discarding one unexecuted** (P7.S3v, after
  P7.S3u). Strict widenings of this design, not redesigns.
- **Inline and static env storage.**
- Polymorphism over plain versus owning quotation types, and owning parameters on spliced or
  generic words. Consequence: no existing higher-order word declared over `[ … ]` can accept an
  owning closure, so a combinator wanting either must be written twice. That type inequality is
  the safety story: it stops an owning closure reaching a combinator that cannot dispose it.
- The REPL's pre-existing inability to link a materialized quotation at all (`__quot0` non-PIC
  relocation), which a REPL owning closure joins rather than creating a new failure class.

## Tests

`tests/phase7_slice3h.rs`, plus unit tests beside each touched function.

Goldens cover: the `bool` capture admitting and a `Copy` struct / `[i64 4]` capture still
rejecting (with the remedy line present only for a linear capture); an owning field and an owning
enum *variant* field rejected by the existing audit; `type: owning …` and `| owning |` reserved;
`owning` in a term position as an unknown word; exactly-once disposal of one and of two linear
captures via a `Spy` fixture with an observable side effect (one observation, not zero or two);
`drop` on an owning closure rejected on both the concrete and generic paths; a one-armed
conditional hitting the pre-existing not-consumed-on-every-path error and a both-arms one running;
a body that only borrows its linear capture rejected, per join arm; a declared `owning` parameter
taking a caller's literal; an owning parameter inherited by an `impl` member lowering to the
quotation aggregate; a spliced `owning` declaration rejected; a non-capturing owning literal
building and running with no allocation.

The env free is guarded by asserting the emitted body **contains a `FREE_SYMBOL` call**, not by a
leak-detecting fixture: a leaked heap block has no observable effect in a normal run and the
harness has no allocator accounting. Mutation direction on the phase-1 predicate is recorded
explicitly, because it is easy to invert: the mutation that matters collapses the arm toward
*admitting*, and the `[i64 4]` rejection is its guard; a collapse toward `FrameRooted` is caught
by the `bool` admit golden.
