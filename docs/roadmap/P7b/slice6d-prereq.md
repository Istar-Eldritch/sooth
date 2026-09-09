# P7b.S6d-PREREQ — Reference-bearing aggregates (condensed reference)

Condensed reference for the shipped capability. The full pre-implementation
plan, with its six adversarial review rounds and per-phase instructions, is
preserved verbatim in [slice6d-prereq-spec.md](./slice6d-prereq-spec.md); this
document keeps only the enduring *why* and *what*, and replaces the *how* with
links to the commits that landed it.

- Companion frozen docs: [slice6d-brief](./slice6d-brief.md) (problem, DQ1–DQ5,
  the four walls) and [slice6d-probes](./slice6d-probes.md) (probe rounds).
- Roadmap entry: [P7b-higher-kinded-types.md](../P7b-higher-kinded-types.md),
  section "P7b.S6d-PREREQ".
- Implementation base: `main` at `486eda4`. Landed on branch `p7b-s6d`:
  - `aa590d2` — Phase 1: declared reference-bearing aggregates (layout, bans,
    seven-site propagation).
  - `613c301` — Phase 2: return-bundle ABI. **Evidence-only** — the ABI already
    existed; Phase 1 supplied the layout gate, member spelling, and pack/unpack
    routing, and `push_dispatch_outputs` already forwarded provenance and
    enforced Ruling F. The deliverable was the golden set, not code.
  - `f27c34d` — Phase 3: roadmap entry, growth re-check.
- Goldens live in `tests/phase7b_slice6d_prereq.rs` (39 tests). They are the
  regression pinning for everything below.

This is a **prerequisite** capability, not an S6d deliverable: it ships no
`Iterator`-for-slice impl. It turns Sooth's silent "you cannot express this"
into a capability plus sharp, located rejections.

## Why

Sooth had no way to put a `Slice[T]`/`!Slice[T]` inside an aggregate. Two rules
jointly enforced this, and together they are the *entire* escape-safety
mechanism in the compiler (there is no lifetime-tracking pass; the closest thing
to a doc is `stored_reference_error`'s own message):

1. `check_no_stored_references` (`check/declarations.rs`) rejects any struct
   field, enum payload, interned array element, or cell payload whose type
   transitively contains a reference. `contains_reference` treats a slice as
   reference-bearing by design: a slice carries a borrow, so a stored slice
   could hand back a view of a dead frame.
2. `check_reference_free_signature` (`check/word_entry.rs`) rejects a non-inline
   word declaring a reference-bearing output, and any input that is not itself a
   reference but contains one nested in an aggregate.

Four walls closed every shortcut past these:

- A `contains_reference` carve-out for `Type::Slice` is unsound by the rule's own
  rationale *and* ICEs at layout (`scalar_size_align_ww` has no path for a
  two-word slice-shaped field).
- An Option-row protocol shape dodges the user-visible rule but not the IR:
  `intern_output_bundles` synthesizes a return-bundle struct for every
  monomorphic word with `out_arity >= 2`, hitting the identical layout refusal.
- A named slice local captured by a quotation reaches the closure-env encoding
  (one word per capture), checker-admitted today and ICE-ing at the backend.
- The poly-body stack checker loses an App-headed dispatch call's outputs, so a
  bound-generic slice consumer fails independently (deferred, see below).

The capability is three-part: **(1)** slice-shaped fields in declared *and*
synthesized aggregates, with real two-word IR layout and backend; **(2)** a
storage-class/taint rule extending the no-stored-reference discipline to the
*containing* value, including an in-frame borrow-propagation extension (Ruling
C); **(3)** — deferred — the poly-body App-dispatch output-loss fix. Landing (1)
without (2) opens exactly the escape the rules exist to prevent.

## Maintainer rulings

Binding rulings from the first-draft review, with implementation-time
amendments. They override any option a reviewer left open.

- **Ruling A (declared-field scope).** Declared aggregates admit **shared
  `Slice[T]` fields only**, under admit-and-taint. **`!Slice[T]` stays
  hard-banned** as a declared field (a located error, plus a golden): `@`, the
  only value-fetch through a field reference, gates on `is_copy` of the referent,
  and `!Slice` is not `Copy`, and Sooth has no move-out-of-a-field mechanism.
  `!Slice[T]` *does* flow through synthesized return bundles (REQ-3), whose
  unpack is positional (no `@`). Readback of a non-`Copy` referent in a declared
  field is deferred.
- **Ruling B (input position).** The input ban is preserved: a reference-bearing
  aggregate may not be an input to a non-combinator word. The capability is
  **body-local** — construct/consume inside a word body, produce across
  boundaries only via inline words (the combinator exemption is retained: a
  spliced word has no frame of its own to escape).
- **Ruling C (in-frame borrow propagation — soundness P0-1).** Every site that
  moves a slice's borrow provenance from one value to another must propagate it,
  or the produced value launders the borrow (**REQ-4d**). The channel is
  **`deriv`** (what `live_derivs`/`live_borrow_of`/`live_mutable_borrow_of`
  read), plus `alias` for the alias-keyed sites. Seven sites, listed below.
- **Ruling D (closure-env boundary: fence, not widen).** A bare
  `Slice[T]`/`!Slice[T]` local, or a slice-bearing aggregate, captured into a
  **materialized** closure is rejected at **every** materialization boundary,
  unconditionally (not gated on `escaping`/`owning`). This fence lands in the
  **same commit** as REQ-2's blit change: without it, the blit turns today's
  capture ICE into silent env-slot corruption (a two-word slice blitted into a
  one-word-per-capture slot). Closure-env slice-slot *widening* is out of scope,
  moot under the fence.
- **Ruling E (slice-element gate stays hard).** `check_slice_element_gate`
  remains an unchanged hard reject: a slice whose element transitively contains a
  reference (e.g. `Slice[Holder]`) is not admitted, even though the struct sweep
  now admits `Holder` itself as a *field* type. This is the safety dependency
  that makes REQ-5's unconditional shared-`Slice[T]` clause sound. The recursive
  shape `type: Holder val Slice[Holder] ;` therefore stays deferred.
- **Ruling F (single-root propagation).** A construction **or field store** whose
  slice-bearing operands carry derivs with distinct `owned_root`s is a located
  error (a two-view-of-two-arrays `Pair` is rejected; so is storing a view of one
  array into an aggregate rooted at another). `Slot.deriv`/`Deriv.owned_root` are
  each single-valued, so a multi-operand aggregate could protect at most one
  root — conservative ban until `Deriv` gains a multi-root representation
  (deferred). Same-root operands keep the first operand's deriv.

## Requirements (what shipped)

- **REQ-1 (two-word slice slot in layout).** Shared `Slice[T]` fields lay out
  through the fixed two-word slice slot (`ptr` at 0, `len` at `word_width`, size
  `2*word_width`, align `word_width`) via a new `Type::Slice` arm in
  `LayoutBuilder::size_align`, mirroring the `Type::Quotation` precedent.
  `scalar_size_align_ww`'s refusal is retained unchanged as the bare-scalar
  backstop. Backend-neutral: every figure is word-width-derived; the `ptr`
  component stays an opaque handle.
- **REQ-2 (projection and codegen, incl. backend).** Construction writes both
  words; `&w` field access reads both as a live `Slice[T]` via `@`; drop is a
  no-op over the slot (`field_is_linear` already sends slice to `false`). Backend
  touchpoints: `member_ty` spells `:{SLICE_TYPE_SYMBOL}`; the four backend
  scalar refusals (`field_load_op`/`field_store_op`, `Instr::Load`/`Instr::Store`
  selection) are **kept** — routing is in the frontend, where `store_field` and
  `slot_value` gain slice arms so a slice field is blit-stored/read as an
  interior-pointer aggregate. `module_has_slice` extended to scan `ir.structs`
  field layouts so a declared-but-never-used slice-bearing struct emits its
  `type :sooth.slice` line.
- **REQ-3 (synthesized aggregates: return bundle only).** Return bundles (mono
  `out_arity >= 2`, plus per-instantiation and splice-record bundles) lay out and
  codegen a slice field through REQ-1/REQ-2. A bundle may carry **both**
  `Slice[T]` and `!Slice[T]` outputs: unpack is positional, so `!Slice`'s
  non-`Copy` referent is never read through a stored reference. Closure-env struct
  is removed from scope (Ruling D's fence means one is never constructed).
- **REQ-4 (the taint rule — reference-bearing containment).** A value
  transitively containing a slice/reference is reference-bearing and inherits
  every existing reference restriction:
  - **(a)** Non-inline output/input ban at `check_reference_free_signature`
    (Ruling B), plus the poly twin `audit_poly_reference_free_signature`, whose
    concrete case (`PolyType::Concrete → contains_reference`) sees a slice-bearing
    aggregate. A declared generic slot is `PolyType::Var` → `false`; there is no
    per-instantiation audit (a named gap, deferred).
  - **(b)** The Ruling D materialization fence in `check_capture_admission`,
    rejecting every slice-bearing capture before any escaping/owning branch, with
    its own one-line message (an IR-encoding limit, not escape).
    `classify_capture` gains a `Type::Variant` aggregate arm.
  - **(c)** Every other construction/storage site (interned array element, cell
    payload, `check_slice_element_gate`, `fill`, `^`, quotation-effect output,
    both word-entry arms) stays unchanged, each re-verified with its own golden.
    One addition: `check_quotation_reference_free_effect` gains an **input** arm
    (its old rationale — "already rejected at declaration" — is falsified for a
    now-admissible shared-slice-bearing aggregate).
  - **(d)** In-frame borrow propagation (Ruling C) — the seven sites below.
- **REQ-5 (field storage: hard-ban → admit-and-taint, shared-`Slice[T]`-only).**
  `check_no_stored_references` relaxes exactly two sweeps (struct field, enum
  payload) for a field whose type is a shared `Slice[T]` or an aggregate
  transitively containing only shared slices. The predicate is recursive and
  normative: `is_shared_slice_bearing(ty) := !contains_reference(ty) || ty is a
  shared Slice[T] || ty is a Struct/Enum all of whose fields satisfy it`. Sound
  only because the slice-element gate (Ruling E) independently rejects a
  reference-bearing slice *element*. `contains_reference` gains a `Type::Variant`
  arm (a monotone widening, not a carve-out — an owning eliminator arm receives a
  `Type::Variant` scrutinee that was otherwise reported reference-free). A
  shared-slice container stays `Copy`; a `!Slice`-bearing bundle is linear but
  needs no destructor over the slot.
- **REQ-6 (every new rejection is located and tested).** The pre-existing
  output-rejection text is reused verbatim (G2/G5 byte-identical). The input arm
  has its own pre-existing text (unlocated, type-naming). Genuinely new
  diagnostics — the capture fence, the two alias/deriv rejections — are located
  and tested; the `!Slice` declared-field ban is already located and unchanged.

## The seven propagation sites (REQ-4d, as shipped)

Every site hands a slice's borrow from one value to another and must propagate
`deriv` (primary) and `alias`. The primary enforcement channel is the
**borrow-side exclusivity scan** (`word_families.rs`: `live_deriv →
conflicting_borrow_error`; `aliasing_origin → aliased_place_borrow_error`), whose
deriv predicate is `d.owned_root.as_deref() == Some(rest)`.

1. **Construction (word-call output push)** — `check/terms.rs`. Forwards the
   first slice-bearing operand's `deriv`/`alias` when the output is an aggregate,
   under Ruling F's distinct-root ban.
2. **`@` fetch arm** — `check/word_families.rs`. Forwards the receiver
   reference's `deriv`/`alias` onto the fetched slot when the referent is
   reference-bearing (else `&w &view @ |s|` re-launders on read).
3. **`&`/`&!` of a provenance-carrying local** — `check/engine.rs`
   (`prov.borrow` delegates to `reborrow` to inherit the local's deriv) **plus**
   the exclusivity scan widened to `... || d.place == rest` in
   `word_families.rs`. Both halves are required together: half (a) alone re-roots
   the borrow and blinds the shared-then-exclusive guard on the local itself.
4. **Poly-call and member-dispatch output pushes** — a **live pre-existing
   unsoundness at HEAD**: a `( 'T -- 'T )` pass-through laundered the deriv, so a
   write through `&!a` was observable through a shared view (`thru` prints `99`).
   Four pushes fixed (`check/poly.rs`: two in `check_poly_call`,
   `resolve_splice_member_call`, `resolve_mono_member_call`).
5. **The `!`/`+!` field store** — an unenumerated write site with zero provenance
   handling. Two rules in `terms.rs`'s existing store provenance block: (i) an
   out-of-frame store of a reference-bearing value is a located error (twin of
   the surviving-set escape guard, for a channel with no surviving set — closes
   the cross-frame stash exploit reachable via Ruling B's top-level-reference
   input exemption); (ii) otherwise the stored value's `deriv`/`alias` joins the
   receiver's root binding, under Ruling F.
6. **The anonymous-receiver projection arm** — `word_families.rs`. Forwarded
   `alias` but dropped `deriv`; now forwards `prov.project(top.deriv)` too. (Round
   2 correctly struck this as not a *construction* site; round 5 found it *is* a
   propagation site.)
7. **Naming a reference-bearing aggregate into a local** — `terms.rs` name-read
   push carried `alias`/`surviving` but no `deriv`, severing the chain. Now
   forwards `scope.local(name).and_then(|b| b.deriv)`. (A reference-typed local
   was already safe via the reborrows path; an aggregate takes the push.)

Two supporting fixes: `dup` retains `top.alias` for a reference-bearing aggregate
(its blit copies the two slice words, so the copy views the same buffer), and the
two conflict guards are widened in opposite directions — `overlapping_projection`'s
`conflicts` closure matches a reference-bearing aggregate slot with `alias:
Some(_)`; `consumed_place_conflict`'s consumed-side exemption is extended to a
reference-bearing aggregate (not treated as a live borrow).

Scope note: *consuming* a `Copy` root while a view is live is legal today and
stays legal (the consume guard is gated on `is_linear`); REQ-4d's in-frame target
is **exclusivity** (`&!` while a view is live).

## Load-bearing invariants (NFRs)

- **NFR-1 (no protocol fork, no carve-out).** No `Type::Slice` carve-out in
  `contains_reference` and no second `Iterator` protocol. REQ-5's relaxation is
  position-local (two named sweeps skipped), never a predicate change. The
  `Type::Variant` arm *strengthens* the predicate, the opposite of the forbidden
  carve-out.
- **NFR-2 (backend-neutral IR).** `Ptr[T]` stays opaque; slice-slot figures are
  word-width-derived. No pointer-as-`u64` assumption.
- **NFR-3 (safety monotonicity).** Every intermediate commit is at least as safe
  as the base. The bans and the capture fence land in the same commit as the
  layout + relaxation, so no state ever admits a slice-bearing value into an
  escaping position. The global layout gate serves declared structs, return
  bundles, and closure-env structs alike, so the bans cannot lag the layout work.
- **NFR-4 (no regression).** Existing pins stay green: the bare-slice
  storage/return bans and `scalar_size_align_ww`'s bare-scalar refusal (slice
  fields route through `LayoutBuilder::size_align` instead). One pin,
  `member_ty_refuses_a_slice`, is deliberately inverted in the same commit that
  gives `member_ty` its slice arm.

## Goldens (regression pinning)

All in `tests/phase7b_slice6d_prereq.rs`.

Phase 1 (layout + backend + all bans + declared-aggregate relaxation):

- **G1** and its enum twin — declared struct/enum slice field builds and runs.
- **G1-input** — slice-bearing aggregate as a non-inline input, rejected (Ruling
  B, unlocated type-naming error).
- **G1-mut-hard-ban** — `!Slice[T]` declared field stays a hard reject (Ruling A).
- **G1-nested** — `type: Outer h Holder ;` admitted and tainted (REQ-5's
  recursive predicate).
- **G2** — non-inline slice-bearing output rejected (unchanged text, = G5).
- **G3** (escaping) + owning twin, and **G-capture-fence** (plain, non-escaping,
  in-frame — the shape that ICEs at `qbe.rs:526` today) — Ruling D's fence.
- **G-alias-ii** (site 1, write side), **G-alias-iii** (site 2, read side),
  **G-alias-iv** (site 6 + `Type::Variant` taint arm; both variants spelled),
  **G-alias-v** (site 7, aggregate naming) — in-frame conflict witnesses.
- **G-distinct-root** (Ruling F) — `Pair` over two arrays rejected, over one
  builds and runs.
- **G-poly-launder** (site 4), two twins: the today-red bare-slice regression
  (`thru` printing `99` must become a rejection) and the aggregate twin. The
  bare-slice twin is the one golden that fails on an unmodified tree.
- **G-store-escape** (site 5 rule i, cross-frame stash) and
  **G-store-distinct-root** (site 5 rule ii, in-frame; same-root store accepted).
- **G-sweep-\*** (REQ-4c, all unchanged rejections): array element, cell payload,
  slice element (over `Slice[&i64]`, per Ruling E), `fill`, `^`.
- **G-quot-effect-output** / **G-quot-effect-input** (the new input arm).
- **G-poly-twin** — concrete slice-bearing poly output rejected (`PolyType::Concrete`).
- **G6** — IR pin: two-word offsets + drop no-op, over a mixed linear+slice struct
  so a mutation deleting the slice handling cannot hide behind the struct's own
  linearity flag.

Phase 2 (return-bundle ABI, evidence-only):

- **G4** — inline `>= 2`-output slice word builds and runs (was the
  `layout.rs:271` panic).
- **G5** — non-inline twin rejected (= G2's message).
- **G-poly-bundle** (four variants) — polymorphic / splice-record /
  slice-bearing-aggregate bundle outputs build and run, spelling the slice member
  and returning by value.

## Deferred / out of scope

- **Poly-body App-dispatch output loss.** The standing S8-era limitation where a
  poly body calling a poly word over a compound receiver loses the call's
  outputs. Independent of layout and taint; its own slice. This is the remaining
  gate for *bound-generic* Iterator consumers over slices.
- **The S6d Iterator-for-slice impl itself** (DQ4 sentinel-substitution
  grounding, the `resolve_mono_member_call` second half, the library impl). This
  spec is the prerequisite only.
- **The instantiation-time poly signature audit.** A non-inline poly word
  `( 'T -- 'T )` instantiated at a slice-bearing type passes both of Ruling B's
  arms (a declared generic slot is `PolyType::Var → false`, and there is no
  per-instantiation audit). The *exploit* is closed as REQ-4d site 4; only the
  signature audit remains deferred. **Note for whoever closes it:** the only
  shape that reaches a genuine bundle pack/unpack carrying a slice is a
  `PolyType::Var` output, so a blanket instantiation-time audit would strand
  REQ-3's ABI behind its own gate — it must leave an **exempt path for
  synthesized multi-output bundles** (the G-poly-bundle goldens die otherwise).
- **Self-tail loops over a tainted aggregate.** Once REQ-4d propagates a deriv
  onto a `Window`, `check_reference_across_back_edge` rejects any argument whose
  deriv has a non-static `owned_root`, so a tainted `Window` can never cross a
  self-tail back edge. Safe but real: S6d's loop-shaped consumers will be
  rejected until a loop-aware borrow story exists. The mask is load-bearing —
  `back_edge_outs` forwards only `surviving` and drops `deriv`, a latent twin of
  site 4's laundering, unreachable only because the guard rejects first. A guard
  test (`back_edge_outs_forwards_surviving_set_along_index_map`) pins the
  rejection so the hole cannot open silently.
- **Multi-root `Deriv`** (Ruling F's lifting condition). Until `Deriv` can
  represent several roots, a `Pair` of windows over two buffers cannot be tracked
  soundly. A borrow-checker design slice of its own.
- **The recursive shape `type: Holder val Slice[Holder] ;`** (Ruling E). Blocked
  by the unchanged slice-element gate; a future slice would have to rule on
  relaxing that gate itself.
- **`!Slice[T]` as a declared field**, a readback story for a non-`Copy` referent
  behind `@`, closure-env slot widening (moot under Ruling D's fence),
  `Slice['T]` generic-element targets (unreachable spelling), and a real
  lifetime/escape tracker (REQ-4 stays a conservative ban pattern).
