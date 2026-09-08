# P7b.S6d-PREREQ — Reference-bearing aggregates

> Companion frozen docs: [slice6d-brief](./slice6d-brief.md) (problem, DQ1–DQ5,
> the four walls, the maintainer ruling of 260907: no protocol fork, no carve-out,
> defer S6d behind this capability) and [slice6d-probes](./slice6d-probes.md)
> (verbatim rounds S6d-1..S6d-4). This spec is the capability slice the S6d roadmap
> entry defers behind; it is a **prerequisite**, not an S6d deliverable, and it
> lands no `Iterator`-for-slice impl itself.
> Base `a9eca84` (worktree `p7b-s6d`; an ancestor of `main` at `7404a71`, nothing
> to rebase). Roadmap entry: [P7b-higher-kinded-types.md](../P7b-higher-kinded-types.md),
> section "P7b.S6d-PREREQ".

## Why

Sooth has no way to put a `Slice[T]` (or `!Slice[T]`) inside an aggregate. Two
rules jointly enforce this, and together they are the *entire* escape-safety
mechanism in the compiler — there is no lifetime-tracking pass anywhere in
`check/` (`declarations.rs:1146` doc, verified in probe round S6d-3):

1. **`check_no_stored_references` (`src/check/declarations.rs:1081`)** rejects any
   struct field, enum payload field, interned array element, or cell payload whose
   type transitively contains a reference. `contains_reference`
   (`src/check/builtins.rs:564`) treats `Type::Slice(..) => true` by design
   (`builtins.rs:564` arm; pinned by `contains_reference_true_for_slice`
   `:1091` and `contains_reference_sees_through_a_struct_field` `:1026`): a slice
   *carries* a borrow, so a stored slice could hand back a view of a dead frame.
2. **`check_reference_free_signature` (`src/check/word_entry.rs:160`)** rejects a
   non-inline word declaring any output whose type contains a reference (it calls
   the same `contains_reference` over `effect.outputs`).

Four walls close every shortcut past these (probe rounds S6d-3/S6d-4):

- A `contains_reference` carve-out for `Type::Slice` (`builtins.rs:564`,
  `=> true` → `false`) is **unsound** by the rule's own rationale — a
  `Step[i64 Slice[i64]]` value could outlive the buffer it views, with no tracker
  to catch it — *and* independently **ICEs at `src/ir/layout.rs:271`**
  (`scalar_size_align_ww`'s `IrType::Slice(_) => unreachable!("a slice value
  resolves via slice_layout, not a scalar")`, pinned by
  `scalar_size_align_refuses_a_slice` `layout.rs:1216`): enum/struct payload layout
  has no path for a two-word slice-shaped field.
- An Option-row protocol shape (remainder as a bare stack value, never packed into a
  user-declared enum field) dodges the user-visible storage rule but not the IR:
  `intern_output_bundles` (`src/check.rs:1192`) synthesizes a return-bundle struct
  for **every** monomorphic word with `out_arity >= 2`, and that struct hits the
  identical `layout.rs:271` refusal. A minimal witness
  (`: two-out inline ( Slice[i64] -- i64 Slice[i64] ) |s| 0 s ;`) panics there.
- A named slice local captured by a quotation builds a closure-env struct with a
  slice field — the same storage class, the same wall.
- The poly-body stack checker loses an App-headed dispatch call's outputs
  (standing S8-era limitation on poly bodies calling poly words over compound
  receivers), so a bound-generic slice consumer fails independently.

The capability, therefore, is three-part: **(1)** slice-shaped fields in declared
*and* synthesized aggregates (real IR layout for a two-word slot); **(2)** a
storage-class/taint rule extending the no-stored-reference discipline to the
*containing* value, so an aggregate holding a slice is itself reference-bearing and
inherits every existing reference restriction; **(3)** — deferred, its own slice —
the poly-body App-dispatch output-loss fix. Landing (1) without (2) opens exactly
the escape the current rules exist to prevent; the phasing below keeps every
intermediate state at least as safe as today.

This turns Sooth's silent "you cannot express this" into a capability plus a set of
sharp, located rejections (CLAUDE.md: diagnostics are behaviour).

## Requirements

Each requirement is independently verifiable against a golden or a unit test.

- **REQ-1 (two-word slice slot in aggregate layout).** Enum-variant payload fields
  and struct fields of type `Slice[T]`/`!Slice[T]` lay out through the fixed
  two-word slice slot (`ptr` at offset 0, `len` at offset `word_width`, size
  `2 * word_width`, align `word_width` — the existing `SliceLayout`,
  `src/ir/types.rs:266`/`slice_layout` `:273`). `scalar_size_align_ww`
  (`src/ir/layout.rs:248`) either gains a `Slice` arm or slice fields resolve
  through a non-scalar slot path (mirroring how `Struct`/`Enum`/`Array`/`Quotation`
  fields already route away from the scalar sizer, `layout.rs:257-270`). The
  `IrType::Slice(_) => unreachable!` refusal at `layout.rs:271` is retained as a
  backstop only if slice fields never reach the scalar sizer; if they may, it is
  replaced by the real layout. **Backend-neutral (CLAUDE.md invariant):** every
  figure is word-width-derived; no `Ptr`-as-`u64` assumption is introduced (the
  slot's `ptr` component stays an opaque handle).

- **REQ-2 (projection and codegen over the slice slot).** Constructing an aggregate
  with a slice field writes both words; projecting the field (`&w` field access,
  eliminator arm binding) reads both words as a live `Slice[T]` value; dropping the
  aggregate is a no-op over the slice slot (a slice's `drop` is already a no-op).
  Tag placement in the containing enum is unaffected by the two-word field.

- **REQ-3 (synthesized aggregates carry slice slots too).** The return-bundle
  struct synthesized by `intern_output_bundles` (`src/check.rs:1192`) for any
  `out_arity >= 2` word, and the closure-env struct synthesized for a quotation
  capturing locals, both lay out and codegen a slice-shaped field through REQ-1/REQ-2.
  An **inline** word with `>= 2` outputs one of which is a slice
  (`: two-out inline ( Slice[i64] -- i64 Slice[i64] ) ...`) builds and runs.

- **REQ-4 (the taint rule — reference-bearing containment).** A value whose type
  transitively contains a slice (or any reference) is **reference-bearing**. The
  no-stored-reference discipline extends to the containing value: a reference-bearing
  aggregate (a) may not be an output of a **non-inline** word, (b) may not be
  captured by a quotation, (c) may not otherwise escape the frame whose borrowed
  storage it holds. This is a **conservative ban**, not an escape-permitting
  lifetime analysis (load-bearing: no lifetime tracker exists — REQ-4 must never
  admit a slice-bearing value into a position that survives its frame). The existing
  `contains_reference` predicate is the taint test; the bans (a) is already enforced
  by `check_reference_free_signature` (`word_entry.rs:160`, over `effect.outputs`)
  and must stay enforced after REQ-5 relaxes field storage; (b)/(c) are enforced at
  the capture / return sites.

- **REQ-5 (field storage: hard-ban → admit-and-taint, atomically with layout).**
  `check_no_stored_references` (`declarations.rs:1081`) stops hard-rejecting a
  slice-typed struct/enum field and instead **admits** it while marking the
  containing type reference-bearing (REQ-4). This relaxation lands **in the same
  phase as** the REQ-1 layout capability and **after** the REQ-4 bans are active, so
  that at no intermediate commit is a slice-bearing aggregate both constructible and
  free to escape. Before this slice: hard reject (fully safe). After: admit, fully
  tainted. There is no intermediate where storage is permitted without the taint
  bans.

- **REQ-6 (every new rejection is a located, tested error — CLAUDE.md).** Each of
  REQ-4(a/b/c) is a located, byte-exact diagnostic (measure-then-pin), naming the
  offending word/aggregate and the reference-bearing field. The pre-existing
  `stored_reference_output_error` text is reused where it already applies (non-inline
  slice-bearing output); the capture/escape bans get house-style messages
  (`thing_condition_expected` naming convention for their tests).

## Non-functional requirements

- **NFR-1 (no protocol fork, no carve-out).** This slice ships **no** `Type::Slice`
  carve-out in `contains_reference` and **no** second `Iterator` protocol. The
  Step-row protocol S8 established works for slices unchanged *once this capability
  exists*; S6d itself is out of scope here (this is the prerequisite only).

- **NFR-2 (backend-neutral IR).** `Ptr[T]` stays opaque; the slice slot's figures
  are word-width-derived (REQ-1). No WASM-blocking `pointer-as-u64` assumption.

- **NFR-3 (safety monotonicity).** Every intermediate commit is at least as safe as
  the base: no state admits a slice-bearing value into an escaping position. The
  phase order (taint bans before / with the layout+relaxation) is the mechanism.

- **NFR-4 (no regression).** Existing goldens stay green: the bare-slice storage/return
  bans (`contains_reference_true_for_slice`, `check_reference_free_signature`'s
  existing pins) remain correct; `scalar_size_align`'s refusal for a *bare* slice
  scalar (not an aggregate field) is unaffected where it still applies.

## Golden tests (phase-exit criteria)

- **G1 (declared struct field).** A struct `type: Holder val Slice[i64] ;`
  constructs from a live slice, projects `val` back to a `Slice[i64]`, iterates it,
  and drops — source in → expected run output. Enum-payload twin
  (`type: Cell | Empty | Full Slice[i64] ;`) alongside.
- **G2 (non-inline return rejected — taint).** A **non-inline**
  `( -- Holder )`-style word returning a slice-bearing aggregate is a located
  error naming the reference-bearing field (`holder_field_condition_expected`).
- **G3 (capture rejected — taint).** A quotation capturing a slice-bearing local is
  a located capture error.
- **G4 (inline ≥2-output slice word).** `: two-out inline ( Slice[i64] -- i64
  Slice[i64] ) |s| 0 s ;` builds and runs (the synthesized return-bundle struct,
  REQ-3) — the exact fixture that panics at `layout.rs:271` today.
- **G5 (non-inline ≥2-output slice word rejected).** The same body without `inline`
  is a located `check_reference_free_signature` error (unchanged text, now reached
  via the aggregate path).
- **G6 (IR pin).** An `emit_ssa_with_manifest`-captured pin that the slice field
  lays out at the two-word offsets and drop is a no-op over the slot.

Unit tests sit beside each changed stage function (`layout.rs` sizer/projection,
`builtins.rs::contains_reference` taint helpers, `declarations.rs::check_no_stored_references`,
`word_entry.rs::check_reference_free_signature`), happy path plus one rejection each,
named `thing_condition_expected`.

## Phases

**Phase 1 — declared reference-bearing aggregates (layout + taint, landed
together).** Add the two-word slice-slot layout for user-declared struct/enum
payload fields (REQ-1/REQ-2): `scalar_size_align_ww` gains a slice arm or slice
fields route through a non-scalar slot path; construction, projection, and no-op
drop over the slot. In the *same* commit, relax `check_no_stored_references`
(`declarations.rs:1081`) from hard-ban to admit-and-taint for slice fields (REQ-5),
with the REQ-4 escape bans active first: keep `check_reference_free_signature`'s
non-inline-output ban (REQ-4a — already via `contains_reference`) and add the
capture/escape bans (REQ-4b/c) as located errors. This ordering is load-bearing
(NFR-3): the taint bans gate every new construction the layout admits. Goldens
G1/G2/G3/G6; unit tests beside the sizer, the taint predicate, and the two checker
gates. Effort L, high difficulty (real IR-layout work plus a soundness-critical
rule change).

**Phase 2 — synthesized reference-bearing aggregates.** Extend the slice-slot
layout and taint reasoning to compiler-synthesized aggregates (REQ-3): the
`intern_output_bundles` return-bundle struct (`check.rs:1192`) and the
quotation-closure env struct. An inline `>= 2`-output word with a slice output
builds and runs (G4); its non-inline twin stays a located error (G5); a capturing
closure stays rejected (covered by Phase 1's REQ-4b, re-witnessed here). This phase
unblocks the ≥2-output `next` shape S6d needs, without landing S6d. Effort M-L, high
difficulty. Re-run growth signals (CLAUDE.md) on `layout.rs` and `check.rs` at phase
exit.

**Phase 3 — evidence, roadmap, growth re-check.** IR pins finalized; the S6d-PREREQ
roadmap entry updated (capability shipped, what remains for S6d: the DQ4 grounding
route and the deferred poly-body fix below); one-line spec link from the entry;
growth-structure re-check recorded on every file this slice grew. Effort S, low.

### Phases (JSON)

```json
[
  {
    "phase": 1,
    "focus": "Declared reference-bearing aggregates: two-word slice-slot IR layout for user struct/enum payload fields (scalar_size_align arm or non-scalar slot path), construction/projection/no-op-drop, landed together with the check_no_stored_references hard-ban to admit-and-taint relaxation and the REQ-4 escape bans (non-inline return, capture, frame-escape) active first",
    "effort": "L",
    "difficulty": "high"
  },
  {
    "phase": 2,
    "focus": "Synthesized reference-bearing aggregates: same slice-slot layout and taint propagation for the intern_output_bundles return-bundle struct and quotation-closure env struct; inline >=2-output slice word builds and runs, non-inline twin and capturing closure stay located errors",
    "effort": "M-L",
    "difficulty": "high"
  },
  {
    "phase": 3,
    "focus": "Evidence, roadmap S6d-PREREQ entry update and spec link, growth-structure re-check on every grown file",
    "effort": "S",
    "difficulty": "low"
  }
]
```

## Deferred / out of scope

- **(3) The poly-body App-dispatch output loss (its own slice, deferred).** The
  poly-body stack checker loses an App-headed dispatch call's outputs (probe round
  S6d-4: a `['It: Cursor]` draining fold fails with a stack-effect mismatch). This
  is the standing S8-era limitation on poly bodies calling poly words over compound
  receivers, **independent** of layout and taint, and is the remaining gate for
  *bound-generic* Iterator consumers over slices. It is not required for the
  capability this spec lands (declared and synthesized slice-bearing aggregates), it
  does not fall out of the layout/taint work, and it looks like its own slice — so it
  is explicitly deferred, not attempted here. S6d (or a dedicated poly-body slice)
  takes it up.
- **The S6d Iterator-for-slice impl itself** — the DQ4 sentinel-substitution
  grounding route, the `resolve_mono_member_call` second half, and the library impl
  over `len`/`&>`/`subslice`/`Step`. This spec is the prerequisite capability only;
  S6d resumes once it lands.
- **A real lifetime/escape tracker.** REQ-4 stays a conservative ban pattern; no
  `outlive`/`dangling` analysis is introduced (NFR-3).
- **`Slice['T]` generic-element targets** — unreachable spelling (S6d-3), never in
  play here.
