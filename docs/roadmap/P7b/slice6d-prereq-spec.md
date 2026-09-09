# P7b.S6d-PREREQ — Reference-bearing aggregates

> Companion frozen docs: [slice6d-brief](./slice6d-brief.md) (problem, DQ1–DQ5,
> the four walls, the maintainer ruling of 260907: no protocol fork, no carve-out,
> defer S6d behind this capability) and [slice6d-probes](./slice6d-probes.md)
> (verbatim rounds S6d-1..S6d-4). This spec is the capability slice the S6d roadmap
> entry defers behind; it is a **prerequisite**, not an S6d deliverable, and it
> lands no `Iterator`-for-slice impl itself.
> Base: `main` at `486eda4` (S8c: per-site binding for member signatures with
> free input type variables). The branch was rebased onto that commit on
> 260909, and `git diff HEAD main -- src/` is empty: `src/` at branch HEAD is
> identical to `main`, so **every citation below is single-anchored at HEAD**
> and needs no branch-versus-main caveat (the round-1..3 dual-anchoring
> apparatus is retired with the rebase). Every `src/` citation was re-verified
> by direct read at `486eda4` on 260909, including the ones S8c shifted
> (`check/poly.rs`, round-5 finding B). **Companion-doc note (round-3 review P2-4):** `slice6d-brief.md` and
> `slice6d-probes.md` are otherwise frozen/verbatim; the one edit either
> received from this loop is a citation correction (`declarations.rs:1074` →
> `:1081`), nothing else.
> Roadmap entry: [P7b-higher-kinded-types.md](../P7b-higher-kinded-types.md),
> section "P7b.S6d-PREREQ".

## Maintainer rulings (260907)

A 3-reviewer soundness/implementability/contract review of the first draft found
two soundness holes and several unbuildable/self-contradictory requirements. The
maintainer ruled as follows; these rulings are binding and override any option a
reviewer left open.

- **Ruling A (declared-field scope).** Declared aggregates (struct fields, enum
  payload fields) admit **shared `Slice[T]` fields only**, under admit-and-taint.
  **`!Slice[T]` remains hard-banned** as a declared field (a located error, unchanged
  from today, plus a golden), because `@` — the only value-fetch through a field
  reference — gates on `is_copy` of the referent (`src/check/word_families.rs:564-567`)
  and `!Slice` is not `Copy` (`src/check/builtins.rs:520`), and Sooth has no
  move-out-of-a-struct-field mechanism. `!Slice[T]` **does** flow through
  synthesized return-bundle structs (REQ-3), whose unpack is positional at the
  return boundary (no `@` involved) — this is all S6d needs (the `!Slice` iterator's
  `Step` remainder travels in the bundle). Designing a readback ruling for
  non-`Copy` referents in declared fields is explicitly deferred.
- **Ruling B (input position).** The input ban is **preserved**: a reference-bearing
  aggregate may not be an input to a non-combinator word
  (`src/check/word_entry.rs:172-179`, the input arm, stays unchanged). The
  capability is **body-local**: construct/consume inside a word body, produce
  across boundaries only via inline words (the combinator exemption,
  `src/check/word_entry.rs:53-55` with its rationale at `:40-52`, is retained
  deliberately — a spliced word has no frame of its own to escape). S6d's consumer
  shape (probe round S6d-4: produce inline, consume in-body) is unblocked by this.
- **Ruling C (in-frame borrow propagation — soundness P0-1, required for
  soundness; channel corrected to `Deriv`-primary per round-3 review P0-1;
  widened to **six sites** per round-5 review P0-1/P0-2/P0-3).**
  New **REQ-4d**: every site that moves a slice's borrow provenance from one
  value to another must propagate it, or the value it produces launders the
  borrow. The six are: aggregate construction (the word-call output push),
  `@` projection, `&`/`&!` of a provenance-carrying local, the poly-call and
  member-dispatch output pushes, the `!`/`+!` field store, and the
  anonymous-receiver projection arm. Two of the six were found by probing the
  live compiler in round 5, and one of them (the poly-call push) is a
  **live pre-existing unsoundness at HEAD**, reproduced end-to-end: see
  REQ-4(d). Both conflict guards are treated per P1-G. The propagating channel
  is **`deriv` (what
  `live_derivs`/`live_borrow_of`/`live_mutable_borrow_of` actually read), plus
  `alias` for the alias-keyed sites**. (Round-2 review P0-A: the original
  citation here for "the
  only alias-set site" — `word_families.rs:445-448` — was wrong; that is the
  anonymous-receiver field-*projection* arm, not a construction site. See
  REQ-4d for the corrected sites: `check/terms.rs`'s output push, the true
  alias mint at `engine.rs:1034`, and the P0-B/P1-G fixes the write-side-only
  framing here missed.) These close an in-frame hole REQ-4a/b/c cannot see (all
  three are boundary bans; this hole is entirely in-frame).
- **Ruling D (closure-env boundary: fence, not widen — 260908).** A bare
  `Slice[T]` (or `!Slice[T]`) local captured into a **materialized** (`[...]`,
  non-`~[...]`) closure is checker-admitted *today*, regardless of
  escaping/owning status, and ICEs at `src/backend/qbe.rs:526` ("an aggregate
  field is copied by blit, not scalar-stored") — reproduced twice independently
  (260908). `classify_capture` (`src/check/captures.rs:224`) sends a bare slice
  down the *borrow* arm (`Type::Ref(..) | Type::Slice(..)`), not the aggregate
  arm (`:212-214`) the round-2 draft's REQ-4b reasoned over — so no existing
  REQ-4b language covers it, and no existing golden witnesses it (G3's
  plain/owning twins are both gated on `escaping`, which is false for an
  in-frame, non-escaping capture like this one). Reproduced fixture (placed
  under `examples/`, deleted after verification): `: use ( [ -- ] -- ) call ;
  : main ( -- ) 0 4 fill | a | &a slice | v | [ v len >i64 . ] use a drop ;` —
  panics at `qbe.rs:526:13` on this worktree today. No working program is
  affected by rejecting it: nothing today constructs this shape (a
  slice-bearing aggregate does not yet exist, and a bare-slice capture already
  ICEs). Therefore: **REQ-4b is extended to reject slice-bearing captures —
  bare slices included — at every materialization boundary**, unconditionally
  (not gated on `escaping` or `owning`), as a single checker fence landing in
  the **same Phase-1 commit** as REQ-2's `field_load_op`/`field_store_op`
  changes. This ordering is load-bearing (NFR-3): without the fence, REQ-2's
  blit change turns today's ICE into **silent corruption** —
  `build_env`'s single-capture path stores the captured value directly into the
  quotation struct's fixed one-word `env` member
  (`src/ir/func_builder/quotation.rs:182`; the quotation aggregate's own type is
  `{ l, l, l }`), and its multi-capture path sizes the env bundle at
  `many.len() * WORD_WIDTH` (`quotation.rs:184`) — one word per capture. Once
  `field_store_op` blits a two-word slice instead of refusing it, both paths
  blit a two-word value into a one-word-per-capture slot, silently overwriting
  whatever follows (a dropped `len`, or a stomped neighboring capture) —
  strictly worse than today's panic. Closure-env slice-slot *widening* (making
  the env encoding itself support a multi-word capture) is **not** part of this
  capability: it is recorded in Deferred as moot under the fence, not a gap
  this phase closes. New golden: **G-capture-fence** — the exact fixture above
  produces a located error. This corrects REQ-3's premise in the round-2 draft
  ("REQ-4b bans capturing a slice-bearing local outright ... closure-env slice
  fields stay unreachable behind REQ-4b"): REQ-4b as drafted there covers
  slice-bearing *aggregates* only; a bare slice capture takes
  `classify_capture`'s borrow arm, not the aggregate arm — that was the gap.
- **Ruling F (single-root propagation — 260908, round-4 review P1; extended to
  field stores per round-5 review P0-2).** A construction **or field store**
  whose slice-bearing operands carry derivs with distinct
  `owned_root`s is a **located error** (a two-view-of-two-arrays `Pair` is
  rejected, and so is storing a view of one array into an aggregate already
  rooted at another); same-root operands keep the first operand's deriv.
  Rationale and
  mechanics in REQ-4(d)'s multi-operand rule; lifted only when `Deriv` gains
  a multi-root representation (Deferred).
- **Ruling E (the slice-element gate stays hard — 260908).**
  `check_slice_element_gate` (`src/check/declarations.rs:1175`, run at `:1069`
  immediately after the relaxed struct/enum sweeps) remains an **unchanged hard
  reject**: a slice whose element type transitively contains a reference (e.g.
  `Slice[Holder]`, where `Holder` holds a shared slice) is **not** admitted,
  even though the struct sweep now admits `Holder` itself as a field type. This
  directly conflicts with the round-2 draft's `G1-recursive`
  (`type: Holder val Slice[Holder] ;` declaring), which requires exactly what
  this gate rejects — both cannot hold. Ruling: the gate wins; **`G1-recursive`
  and REQ-5's recursive-shape sub-bullet are deleted**, and the recursive shape
  moves to Deferred. `check_recursion` (`declarations.rs:1067`/`:1622`) does
  already pass a `Holder val Slice[Holder]` fixture (its containment-graph
  rationale is correct and stays) — the recursion check was never the
  obstacle; the element gate is, and this spec does not touch it.
  `G-sweep-slice-element` keeps its witness by using a directly
  reference-shaped element rather than the round-2 draft's unconstructible
  "`Holder` holding a `&T`" (REQ-5 keeps `&T` fields hard-banned, so that
  `Holder` can never be declared at all): `Slice[&i64]` is a directly
  parseable, user-writable type spelling (`src/parser.rs:7299-7302` interns any
  `Slice[T]`/`!Slice[T]` argument straight from the parser with no shape
  filter, confirmed by `declarations.rs:1160`'s own comment naming
  `Slice[&i64]` as reaching this exact registry) — the golden uses that
  directly, no `Holder` involved.

## Why

Sooth has no way to put a `Slice[T]` (or `!Slice[T]`) inside an aggregate. Two
rules jointly enforce this, and together they are the *entire* escape-safety
mechanism in the compiler — there is no lifetime-tracking pass anywhere in
`check/`; the closest thing to a doc is the rejection message itself
(`src/check/declarations.rs:1140-1148`, `stored_reference_error`: "a `&T`/`&!T`
borrows a local and may not outlive it, so it cannot be put anywhere that survives
the borrow" — a conservative-ban rationale, not an analysis), verified in probe
round S6d-3:

1. **`check_no_stored_references` (`src/check/declarations.rs:1081`)** rejects any
   struct field, enum payload field, interned array element, or cell payload whose
   type transitively contains a reference. `contains_reference`
   (`src/check/builtins.rs:564`, the `Type::Slice(..) => true` arm at `:578`)
   treats a slice as reference-bearing by design (pinned by
   `contains_reference_true_for_slice` `:1091`; `contains_reference_sees_through_a_struct_field`
   `:1026` pins the transitive-struct-field case over a `&`-typed field, not a
   slice — it does not pin the slice arm): a slice *carries* a borrow, so a stored
   slice could hand back a view of a dead frame.
2. **`check_reference_free_signature` (`src/check/word_entry.rs:160`)** rejects a
   non-inline word declaring any output whose type contains a reference (`:167-171`,
   over `effect.outputs`) **and** any input that is not itself a reference but
   contains one (`:172-179`, over `effect.inputs` — Ruling B keeps this arm
   in force unchanged).

Four walls close every shortcut past these (probe rounds S6d-3/S6d-4):

- A `contains_reference` carve-out for `Type::Slice` (`builtins.rs:578`,
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
  for **every** monomorphic word with `out_arity >= 2` (`src/check.rs:1103`, called
  after `check_types` at `:562` — the checker relaxation this spec makes is entirely
  bypassed by this path, so IR layout is the bundle's only gate), and that struct
  hits the identical `layout.rs:271` refusal. A minimal witness
  (`: two-out inline ( Slice[i64] -- i64 Slice[i64] ) |s| 0 s ;`) reproduces the
  panic today (verified 260907, with a driver — see G4).
- A named slice local captured by a quotation reaches the closure-env encoding,
  which is compiler-synthesized and one word per capture — not the
  declaration-time checker sweep at all. A bare slice capture is
  checker-*admitted* today (it takes the borrow arm, not a declared-aggregate
  path) and ICEs at the backend (`qbe.rs:526`) rather than at layout; Ruling D
  closes this with a dedicated capture-boundary fence (REQ-4b), not a layout
  answer — so a slice-bearing capture, bare or aggregate, is rejected before it
  ever reaches a closure-env struct, which needs no slice-slot layout of its own
  (see REQ-3).
- The poly-body stack checker loses an App-headed dispatch call's outputs
  (standing S8-era limitation on poly bodies calling poly words over compound
  receivers), so a bound-generic slice consumer fails independently.

The capability, therefore, is three-part: **(1)** slice-shaped fields in declared
*and* synthesized aggregates (real IR layout for a two-word slot, backend included);
**(2)** a storage-class/taint rule extending the no-stored-reference discipline to
the *containing* value, so an aggregate holding a slice is itself reference-bearing
and inherits every existing reference restriction — **including** an in-frame
borrow-propagation extension (Ruling C, `Deriv`-primary) the first review round
missed; **(3)** —
deferred, its own slice — the poly-body App-dispatch output-loss fix. Landing (1)
without (2) opens exactly the escape the current rules exist to prevent; the
phasing below keeps every intermediate state at least as safe as today.

This turns Sooth's silent "you cannot express this" into a capability plus a set of
sharp, located rejections (CLAUDE.md: diagnostics are behaviour).

## Requirements

Each requirement is independently verifiable against a golden or a unit test.

- **REQ-1 (two-word slice slot in aggregate layout — ruled route).** Enum-variant
  payload fields and struct fields of type shared `Slice[T]` (Ruling A: not
  `!Slice[T]`) lay out through the fixed two-word slice slot (`ptr` at offset 0,
  `len` at offset `word_width`, size `2 * word_width`, align `word_width` — the
  existing `SliceLayout`, `src/ir/types.rs:266`/`slice_layout` `:273`). The route is
  **`LayoutBuilder::size_align`** (`src/ir/layout.rs:726-755`), which gains a
  `Type::Slice(_)` arm computing `(l.size, l.align)` from `slice_layout`, exactly
  mirroring the existing `Type::Quotation` arm (`:750-753`, the precedent).
  **`scalar_size_align_ww`'s refusal at `layout.rs:271` is retained unchanged as
  the bare-scalar backstop** — it is never given a `Slice` arm: that function
  returns `(bytes, bytes)` (`layout.rs:273`) and cannot answer a slice's two-word
  size / word-aligned-not-size-aligned shape (the rationale already in the
  `layout.rs:264-270` comment), and its pin
  (`scalar_size_align_refuses_a_slice`, `layout.rs:1214-1220`, `#[should_panic]`)
  stays green per NFR-4 for exactly this reason. (The earlier draft's "either/or"
  between these two routes is resolved: only `LayoutBuilder::size_align` is
  correct; `layout.rs:257-270`, cited previously as "the routing precedent", are
  in fact the panic arms *inside* `scalar_size_align_ww`, not routing at all.)
  **Backend-neutral (CLAUDE.md invariant):** every figure is word-width-derived;
  no `Ptr`-as-`u64` assumption is introduced (the slot's `ptr` component stays an
  opaque handle).

- **REQ-2 (projection and codegen over the slice slot, including the backend).**
  Constructing an aggregate with a slice field writes both words; projecting the
  field (`&w` field access) reads both words as a live `Slice[T]` value via `@`
  (whose `is_copy` gate, `word_families.rs:564-567`, is why REQ-1 is shared-only
  per Ruling A); dropping the aggregate is a no-op over the slice slot. Named
  backend touchpoints (omitted from the first draft; a worker following only the
  checker/IR requirements panics on the first golden):
  - `member_ty` (`src/backend/qbe.rs:418`, the function that spells a struct's QBE
    member list) currently refuses at `:456`
    (`IrType::Slice(_) => unreachable!("a slice is banned from every field
    position")`); it gains an arm spelling `:{SLICE_TYPE_SYMBOL}` (mirroring
    `qbe_abi_ty`'s existing `IrType::Slice(_) => format!(":{SLICE_TYPE_SYMBOL}")`
    arm at `qbe.rs:411`). This is the emission-ordering-sensitive site: `emit`
    writes the shared `type :{SLICE_TYPE_SYMBOL} = { l, l }` line
    (`qbe.rs:155-157`) **before** the struct-type loop (`qbe.rs:158-160`,
    `emit_struct_type` `:181`), unlike a nested user struct (which needs
    `topo_sorted_structs`, `qbe.rs:203-222`, ordered by containment) — QBE's
    declare-before-reference rule is satisfied because the slice-aggregate
    declaration is unconditionally first, not because of topological order; no
    change to `topo_sorted_structs` is needed. **The existing pin
    `member_ty_refuses_a_slice` (`qbe.rs:2736-2743`) is retired/inverted by
    this change in the same commit** (see NFR-4).
  - `field_load_op` (`qbe.rs:460-498`, its slice arm at `:494`) and
    `field_store_op` (`qbe.rs:500-529`, its slice arm at `:525`) both
    `unreachable!` on `IrType::Slice(_)` (folded into the shared aggregate
    pattern with `Struct`/`Enum`/`Array`/`Quotation`/`OwningQuotation`). A
    second, separate pair of match sites — the `Instr::Load`/`Instr::Store`
    selection (`qbe.rs:1316`/`:1333`) — refuses the same way. **These four
    refusals are KEPT, not given slice arms** (round-4 contract review P1-1):
    they refuse because aggregates never reach them — `Struct`/`Enum`/
    `Array`/`Quotation` are folded into the same `unreachable!`
    (`qbe.rs:490-497`/`:520-529`) — and the routing decision lives in the
    frontend, not the backend. The real change is in the IR builder:
    `store_field` (`src/ir/func_builder/word_families.rs:1046-1066`, aggregate
    arm `:1055-1063`) and `slot_value` (`:1074-1096`, aggregate arm
    `:1084-1088`) gain `IrType::Slice(_)` in their aggregate arms, so a slice
    field is **blit-stored / read as an interior-pointer aggregate value**
    exactly like every other aggregate field, and never reaches the scalar
    arms at all. (Adding scalar arms to the four backend refusals instead
    would store one word of a two-word slice with no diagnostic — the silent
    truncation NFR-3 exists to prevent.) `ir/func_builder` is therefore in
    this slice's unit-test scope (see the test list below); only
    `bind_env_capture` remains out of scope, behind Ruling D's fence.
  - `field_is_linear` (`src/ir/layout.rs:40-59`) already sends `IrType::Slice` to
    the `_ => false` wildcard (`:57`) — **no change needed**; this is the fact that
    makes "drop is a no-op over the slot" true (see REQ-5's linearity note).
  - `module_has_slice` (`src/backend/qbe.rs:262-270`) **must be extended** to scan
    `ir.structs` field layouts, not only `params`/`ret`/`value_types`
    (`build_registries_ww`, referenced via `layout.rs:483-496`, lays out *every*
    declared struct with no reachability pruning, and `emit` writes a `type` line
    for every entry of `ir.structs` unconditionally, `qbe.rs:158-160`). Without
    this, a declared-but-never-constructed-or-called `Holder` (a struct with a
    slice field that never appears in any function's params/ret/value_types) emits
    `type :Holder = { :sooth.slice }` with no preceding
    `type :sooth.slice = { l, l }` line — a QBE undefined-aggregate failure, not a
    Rust panic, so nothing catches it short of a QBE build error.
  - **Three stale comments, falsified by this spec, need updating** (same
    practice as REQ-5's `check_recursion` comment fix). The third is
    `src/check.rs:3262-3265`, `dup`'s "the copy denotes a region of its own"
    rationale, falsified for a slice-bearing aggregate by REQ-4(d)'s `dup`
    rule (round-5 review P1); the other two are backend comments: `qbe.rs:152-154` ("R5
    bans a slice from every field position, so unlike the array/quotation types
    above there is no struct member to declare it ahead of; emission order
    relative to structs is therefore not load-bearing" — falsified now that a
    slice *can* be a field, though the ordering conclusion itself still holds
    for the reason given in this requirement's `member_ty` bullet, not the old
    one) and `qbe.rs:259-261` (`module_has_slice`'s own rationale, "R5 bans a
    slice from every field position, so there is no member-position slice this
    scan needs to (and does not) see" — falsified by the extension above, which
    makes this exactly the scan that now must see one).
  Tag placement in the containing enum is unaffected by the two-word field.

- **REQ-3 (synthesized aggregates: the return-bundle struct only — closure-env
  removed).** The return-bundle struct synthesized by `intern_output_bundles`
  (`src/check.rs:1192`) for any monomorphic `out_arity >= 2` word, **and** the
  per-instantiation bundles for a polymorphic word (`src/check.rs:1111`) and a
  splice record (`:1125`) whose resolved output count is `>= 2`, all lay out and
  codegen a slice-shaped field through REQ-1/REQ-2. Unlike a declared field
  (Ruling A), a bundle may carry **both** `Slice[T]` and `!Slice[T]` outputs: its
  unpack is **positional** at the return boundary (the values are pushed back onto
  the stack in order; no `@` is ever involved, so `!Slice`'s non-`Copy` referent
  is never read *through* a stored reference — it is simply popped). An **inline**
  word with `>= 2` outputs one of which is a slice
  (`: two-out inline ( Slice[i64] -- i64 Slice[i64] ) ...`) builds and runs (G4).
  **The closure-env struct is removed from this requirement, on a corrected
  premise (Ruling D).** The round-2 draft claimed "REQ-4b bans capturing a
  slice-bearing local outright, so a closure-env struct with a slice field is
  never constructed" — false as stated: REQ-4b there covered only slice-bearing
  *aggregates*; a bare `Slice[T]`/`!Slice[T]` local takes `classify_capture`'s
  borrow arm (`captures.rs:224`), not the aggregate arm, and is
  checker-admitted today into a materialized closure, ICE-ing at
  `qbe.rs:526` (verified 260908; the exact fixture is in Ruling D). Ruling D's
  new fence closes this properly: REQ-4b now rejects **every** slice-bearing
  capture — bare or aggregate — at **every** materialization boundary, so a
  closure-env struct with a slice field is never constructed, and there is no
  golden that could witness giving it a slice-slot layout (the contract
  review's P0-3 finding stands, just via the corrected mechanism). Closure-env
  slice-slot *widening* stays out of scope (Deferred, moot under the fence);
  this is a fact to record, not a capability to build.

- **REQ-4 (the taint rule — reference-bearing containment, four named bans).** A
  value whose type transitively contains a slice (or any reference) is
  **reference-bearing** (`contains_reference` is the taint test, and its poly twin
  `contains_poly_reference`, `src/check/audits.rs:352-374` with the concrete case
  at `:360`, is a second enforcement site — see (a)). The no-stored-reference
  discipline extends to the containing value:
  - **(a) Non-inline output/input (word boundary).** Enforced today, unchanged,
    at `check_reference_free_signature` (`word_entry.rs:160`): the output arm
    (`:167-171`) and the **input arm** (`:172-179`, Ruling B — a reference-bearing
    aggregate may not be an input to a non-combinator word either). Both arms are
    **skipped for a combinator** (`word_entry.rs:53-55`, rationale `:40-52`: a
    spliced word has no frame of its own to outlive, so `alpha_rename_locals`
    makes the callee's locals the caller's — retained deliberately, this is the
    mechanism that makes body-local construction/consumption possible at all,
    Ruling B). The **poly twin** is `audit_poly_reference_free_signature`
    (`audits.rs:302-340`, skipped for a combinator the same way, `:312-314`),
    whose concrete case is `contains_poly_reference`'s `PolyType::Concrete(ty) =>
    contains_reference(*ty, ...)` arm (`audits.rs:360`) — this must see a
    slice-bearing aggregate the same way the monomorphic check does. **The
    golden is a declared concrete slice-bearing output** (e.g. `: w ( 'T --
    Window )`), which reaches `PolyType::Concrete` → `contains_reference` → the
    struct arm (`builtins.rs:579-582`) — not a generic output slot *later
    instantiated* at a slice-bearing type: a declared generic output slot is
    `PolyType::Var(_)`, which returns `false` unconditionally at `audits.rs:366`
    (`PolyType::Var(_) | PolyType::Quotation(..) => false`), and
    `audit_poly_reference_free_signature`'s only production caller is
    `audits.rs:289`, over the *declared* signature — there is no per-instantiation
    audit anywhere in the tree (`contains_poly_reference`'s only other caller,
    `poly.rs:3166` on `main` (`:3137` on this branch), guards `^`'s payload
    inside a poly body, not a signature). Instantiation-time checking of a
    slice-bearing output reached only through a generic slot is therefore a
    named gap, not covered by this spec — see Deferred.
  - **(b) Capture (quotation boundary) — a fence at every materialization
    boundary, per Ruling D.** `classify_capture` (`src/check/captures.rs:182-233`)
    sends a slice-bearing struct/enum down the aggregate arm
    (`Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)`,
    `:212-214`) to `CaptureClass::FrameRooted`/`OuterRooted` like any other
    aggregate, and a **bare** `Slice[T]`/`!Slice[T]` local down the separate
    borrow arm (`Type::Ref(..) | Type::Slice(..)`, `:224`). **One arm does need
    changing (round-5 review P0-3): `Type::Variant` is absent from the
    aggregate arm, so an eliminator arm's narrowed scrutinee falls to the
    `_ => CaptureClass::Scalar` wildcard (`:232`)** and a slice-bearing variant
    is snapshotted into the env as if it were an `i64`. `Type::Variant` is
    added to the aggregate arm. The rest of the fix is in
    `check_capture_admission` (`captures.rs:253-345`),
    which admits both today regardless of escaping/owning status (verified
    260908, Ruling D). Two-word slice values cannot fit the one-word-per-capture
    env encoding (`build_env`, `src/ir/func_builder/quotation.rs:175-192`)
    *at all*, independent of whether the borrow they carry would otherwise be
    safe to capture — this is an IR-encoding limit, not an escape-safety
    question, so it applies uniformly to `FrameRooted` and `OuterRooted` alike,
    and to an escaping, owning, or plain (in-frame, non-escaping) boundary
    alike. **Fix (required for NFR-3, Ruling D): `check_capture_admission`'s
    per-name loop (`captures.rs:297-330`) rejects, unconditionally and before
    any escaping/owning branch, any captured name whose type is `Type::Slice(..)`
    directly, or a `Struct`/`Enum`/`Array`/`OwnedCell`/**`Variant`**for which
    `contains_reference` is true** (a bare `Type::Ref(..)` capture is exempt —
    it is one pointer word and already works). **The fence gets its own
    one-line message** (round-3 review P1-5): the rejection reason is an
    IR-encoding limit (a two-word slice value cannot fit the one-word-per-capture
    env encoding), not escape, and the pinned golden case is non-escaping —
    reusing `past_owning_frame_error` (`captures.rs:56-68`, "an escaping
    closure captures …") would pin a message that misstates both the boundary
    and the cause, and its `linear: bool` parameter would suppress the remedy
    line for a shared slice. Only the `(name, span)` plumbing is reused; the
    message text is new and tested (G-capture-fence). This single fence
    subsumes the round-1/round-2 draft's
    narrower "`owning && linear` must exclude `contains_reference`" fix: since
    every declarable slice-bearing aggregate is `Copy` under Ruling A/REQ-5 (so
    never reaches the `owning && linear` branch at all), that scenario's only
    real instance was a bare `!Slice[T]` local captured `owning` — covered by
    the same blanket fence. G3 gains an **owning-closure twin** (today's G3
    draft only pins the plain, escaping capture rejection) and the **new**
    fence golden **G-capture-fence** (Ruling D) pins the plain, *non-escaping*,
    in-frame case neither G3 twin reaches.
  - **(c) Every other construction/storage site — closed enumeration, not an
    open-ended mandate.** These sites already gate on `contains_reference` (or its
    `is_copy`/element-gate siblings) and **stay unchanged**, each re-verified
    against a slice-bearing aggregate with its own rejection golden:
    `declarations.rs:1117` (interned array element), `:1126` (interned cell
    payload — the *only* heap-storage guard: `contains_reference` deliberately does
    not follow `Type::OwnedCell` payloads, "a cell may close a type cycle", so this
    sweep alone keeps a slice out of heap storage), `:1175`
    (`check_slice_element_gate`, the slice-element gate — **stays hard-banned
    outright, Ruling E**: a slice whose element transitively contains a
    reference beyond a shared slice is rejected even though the struct sweep
    above now admits that same element type as a *field*), `src/check.rs:478`
    (`fill`'s element gate), `src/check/word_families.rs:1080` (`^`'s payload
    gate), `src/check/captures.rs:516`
    (`check_quotation_reference_free_effect`'s output arm), and both
    `word_entry.rs` arms named in (a). This closed list replaces the earlier
    open-ended "may not otherwise escape the frame". **One addition, not a
    re-verification:** `check_quotation_reference_free_effect` has no
    **input** arm today (`captures.rs:495-525`), on the rationale (`:500-503`)
    that "an aggregate input carrying a nested reference is already rejected
    at its struct/array declaration" — REQ-5 falsifies that rationale for a
    shared-slice-bearing aggregate (it is no longer rejected at declaration),
    so this boundary needs its own input arm, mirroring the word-level one
    (`word_entry.rs:172-179`).
  - **(d) In-frame borrow propagation (Ruling C, new — closes the P0-1 soundness
    hole; channel corrected per round-3 review P0-1; **six** sites per round-5
    review P0-1/P0-2/P0-3).** Every site that hands a slice's borrow from one
    value to another must propagate its provenance, or the produced value
    launders the borrow. **The site list is the requirement**: five of the six
    were found by successive review rounds probing the live compiler, and a
    missed one is not a missing nicety but an accepted program that violates
    exclusivity. The channels are:
    - **The primary enforcement channel is `Deriv`, not alias sets.** The
      guards that fire for REQ-4d's goldens are the **borrow-side exclusivity
      scan** (`word_families.rs:242-262`: `live_deriv` →
      `conflicting_borrow_error` at `:243`; `aliasing_origin` →
      `aliased_place_borrow_error` at `:268-271`, the alias-keyed sibling),
      whose deriv predicate is `d.owned_root.as_deref() == Some(rest)`
      (`:242-245`) — both G-alias goldens take the view first and the second
      `&!` second, so this is the scan that runs. The deriv-keyed readers
      beyond it: `live_derivs` (`src/check/engine.rs:958-972`) scans
      `slot.deriv`/`b.deriv`; `live_borrow_of` (`:991`) and
      `live_mutable_borrow_of` (`:1005`) consume it — but the consume guard
      (`terms.rs:258`) is gated on `is_linear` and the naming guard
      (`terms.rs:283`) fires only when *naming a local* while a mutable
      borrow lives (the opposite order to the goldens), so neither is these
      goldens' firing path (round-4 review P1: the round-3 close-out
      misattributed them). Alias sets are consulted at exactly two places
      (`word_families.rs:437`, the anonymous-receiver projection arm's
      `overlapping_projection` call, and
      `:527`, `consumed_place_conflict`). Probed 260908: the bare-slice
      analogue `&a slice |s|  &!a |r|  ...` is rejected via the deriv path —
      so a propagation that forwards only `alias` ships inert. **REQ-4d
      therefore propagates `deriv` as the primary channel, and `alias`
      alongside it for the alias-keyed sites; the borrow-side exclusivity
      scan (`word_families.rs:243`/`:268`) is added to the enforcing-reader
      list.**
    - **Site 1, write side (construction) — the word-call
      output push, `src/check/terms.rs:1194-1198`** (a struct/enum
      constructor is a generated `env` word, so its output goes through the
      ordinary word-call path): `stack.push(Slot { surviving, variant_idx:
      nullary_variant_idx, ..Slot::computed(*ty) })`
      already forwards `surviving` — folded from carried inputs by
      `prov.union_surviving` (the `carried` fold, `terms.rs:1163-1164`) — but
      drops both `deriv`
      and `alias` (`Slot::computed`'s defaults, `check.rs:328-340`). This site must forward the
      slice-bearing operand's `deriv` (e.g. via `Slot::derived`, `check.rs:345`) **and** its
      `alias`, the same way `surviving` is folded. **Multi-operand rule
      (round-4 review P1, ruled — Ruling F):** `Slot.deriv` is a single
      `Option<DerivId>` (`src/check.rs:312`) and `Deriv.owned_root` a single
      `Option<String>` (`engine.rs:63`, struct at `:56-84`); `union_surviving` (`engine.rs:343-355`)
      has no deriv analogue and none can be written without a multi-root
      `Deriv`. So "derivs mirroring the surviving fold" is unimplementable as
      stated, and a two-slice-field aggregate (`type: Pair a Slice[i64] b
      Slice[i64] ;`, legal under REQ-5) built from views of two *different*
      arrays could protect at most one root. **Ruling: a construction whose
      slice-bearing operands carry derivs with distinct `owned_root`s is a
      located error** (conservative ban — the alternative silently leaves the
      second root re-borrowable, the exact laundering hole at arity 2), with
      its own golden; same-root operands (two views of one array) keep the
      first operand's deriv, the choice immaterial. The alias side merges via
      `prov.alias_union` (`src/check/engine.rs:239`); the merged `Alias`'s
      `span` is the **first slice-bearing operand's** (ruled here, since the
      span is what `AliasOrigin::Stack` reports). Lifting this restriction
      requires a multi-root `Deriv` — recorded in Deferred.
      (Round-2 review P0-A: the round-1 draft named `word_families.rs:444-447`
      as "the only alias-set site", which is in fact the
      *anonymous-receiver field-projection* arm — its own comment at
      `word_families.rs:427-434` says so — not a construction site at all,
      though site 6 below shows it *is* a propagation site; the
      real alias **mint** is `projected_region` at `src/check/engine.rs:1023-1042`,
      `parent.alias = Some(Alias { set, span })` at `:1034`, reached from the
      owned-receiver forward at `word_families.rs:420` and the anonymous-receiver
      arm at `:435-447`.)
    - **Site 2, read side (`@`'s fetch arm) — round-2 review P0-B.** `@`'s fetch arm
      (`word_families.rs:583-587`) pushes `Slot { surviving, ..Slot::computed(referent)
      }` — it forwards `surviving` (computed just above at `:578-582`) but drops
      both `deriv` and `alias` unconditionally, even though the receiver
      reference's `deriv` is in hand at `:578-582`. Left unfixed, `&w &view @
      |s|` (G1's own idiom) re-launders the borrow on read: `s` binds a fetched
      `Slice[i64]` invisible to every guard, even after the write-side fix
      propagates provenance onto `w`. **Fix: `@`'s fetch arm must forward the
      receiver reference's `deriv` (primary) and `alias` onto the fetched slot
      when the referent is reference-bearing**, mirroring the `surviving`
      forward directly above it. New golden **G-alias-iii** (re-spelled per
      round-3 review P0-2 — see below): taking a `&!` of the root while an
      `@`-projection of the field is live is rejected.
    - **Site 3, `&`/`&!` of a provenance-carrying local
      (round-4 review P0).** `prov.borrow` (`src/check/engine.rs:364-380`,
      called from `word_families.rs:274`)
      sets `owned_root: Some(place.to_string())` (`engine.rs:373`) from the
      borrowed local's *name* and
      discards any deriv the borrowed local's binding carries — unlike
      `reborrow` (`:384-402`), which inherits `held`'s `owned_root` (`:391`).
      Without a
      fix here the chain severs at `&w`: `&w` mints a deriv rooted at `"w"`,
      `@`'s forwarding hands `s` a deriv rooted at `"w"`, and the second-`&!`
      scan (whose predicate is `d.owned_root.as_deref() == Some(rest)`,
      `word_families.rs:244`) looks for `"a"` and finds nothing —
      G-alias-iii goes green and the read-path hole survives. **Fix:
      `prov.borrow` inherits the borrowed local's binding deriv's
      `owned_root` when it has one** (exactly `reborrow`'s `held` logic at
      `:391`), with a unit test beside it. Over-rejection risk is bounded:
      this only extends tracking along chains that already carry a deriv, in
      the same direction `reborrow` already does; any existing golden that
      goes red under it is a real signal, surfaced at implementation time.
    - **Site 4, the poly-call and member-dispatch output pushes — a LIVE
      pre-existing unsoundness at HEAD, not a future risk (round-5 review
      P0-1).** A generic pass-through launders the deriv today. Reproduced
      end-to-end at `486eda4` on 260909:

      ```sooth
      : thru ( 'T -- 'T ) ;

      : main ( -- )
        0 4 fill | a |
        &a slice thru | s |
        &!a | r |
        r 0 >usize &!> 99 !
        s 0 >usize &> @ .
        a drop
      ;
      ```

      This **builds and prints `99`**: a write through the exclusive `&!a` is
      observed through the shared view `s`. Deleting `thru` from the same
      program restores the rejection ("`&!a` conflicts with a live borrow of
      `a`"), so the pass-through call is precisely what launders it. The cause
      is that every dispatch path truncates the operands and re-pushes outputs
      as `Slot::computed`, which zeroes `deriv` and `alias`:
      `check_poly_call`'s two pushes (`src/check/poly.rs:7955-7957` and
      `:7994-7996`), `resolve_splice_member_call`'s
      (`poly.rs:2090-2092`), and `resolve_mono_member_call`'s
      (`poly.rs:2614-2616` — a fourth push found by re-verifying the site list
      at HEAD, not named in the round-5 report; a worker must fix all four or
      the hole survives on the monomorphic-impl dispatch path).
      `check_reference_free_signature` never fires on `thru` because a
      declared `'T` slot is `PolyType::Var`, which `audits.rs:366` returns
      `false` for unconditionally.
      **Fix: all four pushes forward the slice-bearing operands' `deriv`
      (primary) and `alias` onto the outputs, under Ruling F's single-root
      rule verbatim.** Note the bare-slice form above needs no new capability,
      so this is a today-red regression test (G-poly-launder's twin) that the
      same one-line-per-site forward closes; `Window` merely widens the same
      hole to aggregates.
    - **Site 5, the `!`/`+!` field store — an unenumerated write site
      (round-5 review P0-2).** The store's type-check arm
      (`word_families.rs:589-642`) has **zero** provenance handling: it checks
      quotation-ness, `ref_parts`, mutability, `is_copy` of the referent
      (`:622-626`), the `+!` integer rule and `match_slot`, then truncates
      (`:642`). A shared `Slice[T]` referent passes the `is_copy` gate
      (`builtins.rs:520`, `Type::Slice(_, mutable, _) => !mutable`). Two
      exploits follow once REQ-5 admits a slice field:
      **(a) in-frame root swap.** Construct a `Window` from root `b` (legal,
      single-root), then `&!w &!view  &a slice  !`. The `Window` now views `a`
      while its propagated deriv still says `b`: Ruling F is bypassed, and
      site 2's `@` forward then propagates the **wrong** root, which is worse
      than propagating none.
      **(b) cross-frame stash.** Ruling B's input ban exempts a top-level
      reference (`word_entry.rs:173`, `!slot.ty.is_ref() && ...`), so a
      **non-inline** word may take `&!Window` and store into its field a view
      of its own frame's array. Verified at HEAD on 260909 with an `i64`
      field standing in for the slice (`: stash ( &!Window -- ) |w| 0 4 fill
      |own| ... w &!view 7 ! own drop ;`, caller reads `7`): the shape is
      reachable today, and with a `Slice[i64]` field the stored value is a
      view of storage that dies at `stash`'s frame exit. This is a dangling
      view produced without crossing any REQ-4a/b/c boundary, because the
      reference input is exactly the exemption those bans grant.
      The pre-existing out-of-frame escape guard on stores
      (`terms.rs:478-481`, via `ref_root_is_in_frame`, `captures.rs:140-152`)
      does not catch it: it is gated on `stack[vi].surviving` being `Some`
      (`:478`), and a slice value carries no surviving set.
      **Fix: the `!`/`+!` store gains two rules**, placed in `terms.rs`'s
      existing `!`/`+!` provenance block (`terms.rs:449-510`), not in the
      type-check arm, because the machinery is already there:
      (i) if the stored value is reference-bearing and the receiver's root is
      not in-frame (`ref_root_is_in_frame` false), a located error with its
      own one-line message — the exact structural twin of the surviving-set
      escape guard at `:478-481`, extended to a channel that has no surviving
      set; (ii) otherwise the stored value's `deriv`/`alias` **joins the
      receiver's root binding**, under Ruling F's distinct-root rule (storing
      a view of a different root into an already-rooted container is the same
      located error). The mechanics have a precedent to copy verbatim:
      `terms.rs:500-508` already looks the root binding up through
      `deriv → owned_root → scope.local` and unions the stored value's
      surviving set onto it (rationale at `:495-499`), and `@`'s fetch arm
      reads the same channel back out at `:578-582`. The new rule is that
      join, over `deriv`/`alias` instead of `surviving`.
    - **Site 6, the anonymous-receiver projection arm (round-5 review P0-3).**
      `&f`/`&!f` off an **anonymous** receiver (`word_families.rs:426-448`)
      pushes `Slot { alias: Some(alias), ..Slot::computed(out) }` (`:444-447`):
      it forwards `alias` but **drops `deriv`**, unlike the owned-receiver arm
      directly above it, which forwards `prov.project(top.deriv)` (`:415`) and
      `top.alias` (`:420`). Since `deriv` is the primary enforcement channel,
      the anonymous path is alias-only and therefore weaker on exactly the
      shape site 2 relies on. **Fix: the anonymous arm forwards
      `prov.project(top.deriv)` alongside the alias it already carries.** The
      irony is worth recording so no later round re-litigates it: round 2
      correctly struck this site as *not a construction site*, and round 5
      finds it *is* a propagation site.
    - **`dup` must not void the alias channel for a reference-bearing
      aggregate (round-5 review P1).** `dup` pushes
      `Slot { alias: None, ..top }` (`src/check.rs:3266`) on the rationale
      (comment, `check.rs:3262-3265`) that "`dup` of an aggregate deep-copies
      it (`Alloc`+`Blit`), so the copy denotes a region of its own". That is
      **false for a slice-bearing aggregate**: the blit copies the two slice
      words, so the copy views the same buffer as the original. `deriv`
      survives `dup` (only `alias` is cleared), so there is no full exploit
      today, but the alias half of every rule above is voidable by one token.
      **Rule: `dup`'s alias reset is retained only for a non-reference-bearing
      aggregate; a reference-bearing one keeps `top.alias`.** The stale
      comment is listed for update in REQ-2.
    - **Both conflict guards, widened in opposite directions (round-2 review
      P1-G — read carefully, these are two different fixes, not one).**
      `overlapping_projection`'s `conflicts` closure (`word_families.rs:477-488`,
      the inner top-level-only match at `:483-486`) tests the *other* slots on
      the stack/in scope for a live alias, and must be **widened** to also match
      a reference-bearing aggregate slot/binding with `alias: Some(_)`, not only
      a top-level `Type::Ref`/`Type::Slice` (today's `:483-484` match) — this is
      what makes packing a `Slice[i64]` into a `Holder` visible to the *other*
      side of a conflict check. `consumed_place_conflict` (`:516-528`) is the
      opposite side: its `Type::Ref(..) | Type::Slice(..)` early-return match at
      `:524` is a **consumed-side exemption** (rationale at `:512-515`, "a
      reference being consumed is not a place ending") — a reference-bearing
      aggregate being consumed is likewise not a place ending, so `:524`'s
      exemption must be **extended** to also exempt a reference-bearing
      aggregate, not widened to treat one as a live borrow (widening this side
      instead would spuriously reject a legal drop of a `Holder` whose view has
      already gone dead). The two sites take opposite fixes because they answer
      opposite questions: `conflicts` asks "is this *other* slot a live borrow
      of the alias I'm about to touch", `consumed_place_conflict` asks "does
      *consuming this* end a place". Both interact with `live.dead` liveness
      (`conflicts` skips a dead binding via `!live.dead(&b.name, at)`,
      `word_families.rs:492`; `dead` is last-use, `engine.rs:767-771`) — see the
      goldens' liveness note below.
    - Without any of the write/read/guard fixes, packing a live slice into a
      `Holder` inside one word body (never crossing any REQ-4a/b/c boundary)
      strands the checker: a second `&!` borrow can be taken while a view is
      live inside the `Holder` (write-side gap), and a field read through `@`
      re-launders the borrow (read-side gap). **Scope note (round-3 review
      P0-2, probed 260908):** *consuming* a `Copy` root while a view of it is
      live is **already legal today and stays legal** — `0 4 fill |a| &a slice
      |s| a drop s len >i64 .` builds and prints `4`, because the consume
      check (`terms.rs:255-268`) is gated on `is_linear(consumed)` and a
      frame-local array's storage outlives the body anyway. REQ-4d therefore
      makes no consume-of-Copy-root claim; its in-frame target is
      **exclusivity** (`&!` while a view is live), and the consume guard
      reaches through an aggregate only for a **linear** root, via the deriv
      propagation above.
    - **Goldens, spelled to actually witness the mechanism (round-2 review
      P1-H/NEW-3; re-channeled per round-3 P0-1/P0-2):**
      `overlapping_projection`'s `conflicts` closure skips any *dead* binding
      (`live.dead`, last-use semantics, `engine.rs:767-771`) — the same
      mechanism that keeps G1 legal (`w`'s last use, `w total`, precedes
      `a drop`, so `w` is dead by then and no conflict fires). The surviving
      in-frame goldens are **G-alias-ii** (write side: pack a shared slice,
      then a second `&!` of the root while the `Holder` is live and read
      afterward) and **G-alias-iii** (read side: `@`-project the field, then a
      second `&!` while the projection is live) — both are borrow-conflict
      witnesses that actually go red under the deriv propagation; the earlier
      consume-the-root spellings were probed to be inert accepts (see the
      scope note above) and deleted.
    - **G-alias-ii is restated over a *shared* slice (round-2 review P0-C):**
      the round-2 draft's "a second `&!` borrow while a `!Slice`-bearing value
      packed into an aggregate is live" is unconstructible under Ruling A —
      `!Slice[T]` is hard-banned as a declared field, and the only other
      `!Slice`-bearing aggregate is the synthesized return bundle (REQ-3),
      which is interned after `check_types` (`check.rs:1103`/`:562`) and never
      appears as a checker-stack value inside a word body at all, so there is no
      point where a `!Slice`-bearing aggregate is live and checkable. Restated:
      a **shared** slice packed into a `Holder`, then a second `&!` borrow of
      the root while the `Holder` is live (and read afterward, per the liveness
      note above), is rejected — the widened `conflicts` closure still fires,
      since it returns true on `mutable || other_mutable`
      (`word_families.rs:487`), so the new `&!`'s own mutability is enough.

- **REQ-5 (field storage: hard-ban → admit-and-taint, position-local, atomic with
  layout, shared-`Slice[T]`-only).** `check_no_stored_references`
  (`declarations.rs:1081`) relaxes **exactly two** of its four sweeps — the struct
  field sweep (`declarations.rs:1089`) and the enum payload sweep (`:1101`) — and
  only for a field whose type is a **shared** `Slice[T]`, or an aggregate that
  **transitively contains only shared slices and no other reference** (the
  admit-and-taint predicate, **corrected** per round-2 review P0-F — the
  round-2 draft's "all of whose fields satisfy `is_shared_slice_bearing`" has
  no base case for a reference-free field and so rejects the spec's own `Window`
  fixture, whose `lo` field is a bare `usize`: `is_shared_slice_bearing(ty) :=
  !contains_reference(ty)` (a plain reference-free type, admitted trivially)
  `|| ty` is a shared `Slice[T]`, `|| ty` is a `Struct`/`Enum` all of
  whose fields (resp. non-empty variant payloads) satisfy
  `is_shared_slice_bearing`). **The recursive form is
  normative** (round-3 review P1-2): the earlier "equivalently, reject iff the
  type transitively contains a `&T`, `&!T`, or `!Slice[T]`" restatement was
  **not equivalent** — it would reject `Slice[&i64]`/`Slice[!Slice[i64]]`,
  which the recursive form admits at the struct sweep (they are then caught by
  the element gate, see below) — and it is deleted. **Safety dependency,
  stated:** the unconditional shared-`Slice[T]` clause is sound only because
  `check_slice_element_gate` (`declarations.rs:1175`) stays a hard reject
  (Ruling E) and independently rejects a reference-bearing slice *element* —
  a field typed `Slice[&i64]` passes the relaxed struct sweep and is rejected
  one declaration-check later by that gate (with a less-located,
  element-naming message; accepted trade-off, Ruling E). The predicate's
  `Array` arm is **unreachable** (round-3 review P2-3/soundness P2-4): an
  array type whose elements satisfy the predicate is still rejected at the
  array sweep (`declarations.rs:1117`, unchanged per REQ-4c) before any
  struct field of that type could be admitted — retained in the definition
  only for symmetry, and a worker must not read it as an admission path.
  This still rejects a field typed `&T`/`&!T` directly or transitively, and
  rejects `!Slice[T]` outright per Ruling A).
  - **`contains_reference` gains a `Type::Variant` arm (round-5 review P0-3 —
    a monotone widening of the taint test, not a carve-out).**
    `contains_reference` (`builtins.rs:564-593`) has arms for `Ref` (`:571`),
    `Slice` (`:578`), `Struct` (`:579-582`), `Enum` (`:583-587`) and `Array`
    (`:588-590`), and **no `Type::Variant` arm**, so a variant type falls to
    `_ => false` (`:591`) and is reported reference-free. `Type::Variant`
    (`src/ast.rs:3161`) is what an owning eliminator arm receives: `check.rs`
    narrows the scrutinee to the matched variant and hands the arm
    `Slot { ty: narrowed, ..scrutinee }` (`check.rs:2593-2595`), which
    correctly inherits the scrutinee's full provenance, but the *type* it
    carries is invisible to every consumer of the taint test. With Ruling A's
    enum payloads admitted (G1's enum twin), an arm-bound slice-bearing
    payload is therefore untracked end to end. **Fix: a
    `Type::Variant(id, vi, _)` arm testing that variant's own fields**, the
    exact shape of the existing `Enum` arm restricted to one variant.
    This **strengthens** the predicate (more types report `true`), so **NFR-1
    holds**: it is the opposite of the `Type::Slice => false` carve-out NFR-1
    forbids, and it adds no position-local skip. **Implementation instruction:
    the worker must walk every call site of `contains_reference` before Phase 1
    lands** (it is the predicate "every escape rejection is stated over", per
    its own doc comment) and record the result. The only risk of a monotone
    widening is **over**-rejection, which is safe-by-default and surfaced by
    the existing goldens; a call site that must keep the old answer is a
    finding to raise, not to paper over.
  Concretely: `type: Outer h Holder ;` (nesting a
  slice-bearing struct inside another struct) is **admitted and tainted**, per
  REQ-4's "transitively contains" containment model — the naive "reject unless
  the field type is exactly `Type::Slice`" would wrongly reject this composition,
  since `contains_reference(Holder)` is already `true` via the struct arm
  (`builtins.rs:579-582`); NFR-1's "no `contains_reference` carve-out" is satisfied
  because the relaxation is **position-local** (skipping the check at two named
  call sites), not a predicate change. **Unchanged hard rejects** — enumerated,
  not left implicit — are the other two `check_no_stored_references` sweeps
  (array element `:1117`, cell payload `:1126`) plus the five sites named in
  REQ-4(c). This relaxation lands **in the same phase as** the REQ-1 layout
  capability and **after** the REQ-4 bans are active, so that at no intermediate
  commit is a slice-bearing aggregate both constructible and free to escape.
  Before this slice: hard reject (fully safe). After: admit-and-taint for shared
  slices only, fully tainted otherwise. There is no intermediate where storage is
  permitted without the taint bans.
  - **The recursive shape (`type: Holder val Slice[Holder] ;`) stays deferred,
    not newly admitted (Ruling E, superseding the round-2 draft's
    "newly-declarable, ruled admitted" bullet and its `G1-recursive` golden —
    both deleted).** `check_slice_element_gate`
    (`declarations.rs:1175`, REQ-4(c), unchanged) rejects `Slice[Holder]`
    outright, since `contains_reference(Holder)` is `true` via the struct arm
    (`builtins.rs:579-582`) once `Holder` itself is admitted as a field type —
    this fires regardless of what REQ-5's two named sweeps relax. `check_recursion`
    (`declarations.rs:1622`) does already pass a `Holder val Slice[Holder]`
    fixture (`Type::Slice(..) => None` at `:1684-1688` closes no by-value
    containment edge), so its comment ("the same no-stored-reference rule keeps
    it out of every field position too") is still updated to say the field is
    admitted as a *struct field* rather than excluded — but the recursion check
    passing was never what stood between this shape and existing: the slice-element
    gate is, and this spec does not relax it (Ruling E). Recorded in Deferred.
  - **Linearity asymmetry (one line, per NFR-4).** A shared-slice-only container
    stays `Copy` (Ruling A scope). Had a `!Slice` field been admitted to a declared
    struct, its container would become checker-**linear** (`is_linear = !is_ref &&
    !is_copy`, `builtins.rs:545-551`; a struct with a non-`Copy` field is not
    `is_copy`) — move-tracked, owing an explicit `drop` — moot for declared fields
    under Ruling A but true for a return-bundle carrying a `!Slice` output (REQ-3).
    IR's `field_is_linear` (`layout.rs:40-59`) sends every `IrType::Slice` variant
    to `_ => false` regardless of mutability, so no destructor is synthesized for
    the slot either way — REQ-2's "drop is a no-op over the slice slot" claim is
    verified correct for both the shared and (bundle-only) mutable case.

- **REQ-6 (every new rejection is a located, tested error — CLAUDE.md; existing
  text is reused verbatim, not "upgraded").** The pre-existing
  `stored_reference_output_error` (`builtins.rs:603-608`) is called with an empty
  `location` from the word-level check (`word_entry.rs:169`, "unlocated, matching
  its sibling calls", `builtins.rs:598-601`) and names the offending **type**, never
  a field. REQ-4(a)'s non-inline **output** rejection **keeps this text verbatim
  — G2 and G5's message are byte-identical**, both reached via the same call site,
  now over an aggregate type instead of a bare reference type. **This
  "verbatim" claim is confined to the output rejection (round-2 review NEW-4):
  the input rejection is a separate, pre-existing arm with its own text** —
  `word_entry.rs:175-178`'s own inline `format!` ("error: a reference cannot be
  stored: ... declares the input `{ty}`, which contains a reference ... an
  input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate"),
  never a call to `stored_reference_output_error`. **G1-input is
  pinned to this input-arm text**, described as an unlocated, type-naming error
  (same class as G2/G5, not a located one — correcting the round-2 draft's
  "located error" description of this golden). The poly twin's own input text
  (`audits.rs:333-336`) is the same class, over `poly_type_str` instead of
  `Type`. The "located, field-naming, byte-exact" bar in this requirement is
  confined to the genuinely **new** diagnostics this spec introduces: REQ-4(b)'s
  capture-fence rejection (**its own new one-line message** — round-3 review
  P1-5: an IR-encoding rejection, not an escape rejection, so
  `past_owning_frame_error`'s text is NOT reused; only its `(name, span)`
  plumbing is), REQ-4(d)'s two alias/deriv rejections (via the existing
  `conflicting_projection_error`/`consume_of_borrowed_place_error` located
  machinery), and the `!Slice`
  declared-field hard ban (already located and unchanged — `stored_reference_error`,
  `declarations.rs:1140-1148`, already carries a field name and a span). Naming
  convention for their tests: `thing_condition_expected`.

## Non-functional requirements

- **NFR-1 (no protocol fork, no carve-out).** This slice ships **no** `Type::Slice`
  carve-out in `contains_reference` and **no** second `Iterator` protocol. The
  Step-row protocol S8 established works for slices unchanged *once this capability
  exists*; S6d itself is out of scope here (this is the prerequisite only). REQ-5's
  relaxation is position-local (two named call sites skipped), never a predicate
  change — this is what keeps NFR-1 true after the relaxation.

- **NFR-2 (backend-neutral IR).** `Ptr[T]` stays opaque; the slice slot's figures
  are word-width-derived (REQ-1). No WASM-blocking `pointer-as-u64` assumption.

- **NFR-3 (safety monotonicity).** Every intermediate commit is at least as safe as
  the base: no state admits a slice-bearing value into an escaping position, in- or
  cross-frame. The phase order (taint bans before / with the layout+relaxation,
  landed in one phase — see Phase 1) is the mechanism; REQ-4d closes the in-frame
  gap the boundary bans (a/b/c) cannot see.

- **NFR-4 (no regression).** Existing goldens stay green: the bare-slice storage/return
  bans (`contains_reference_true_for_slice`, `check_reference_free_signature`'s
  existing pins) remain correct; `scalar_size_align_ww`'s refusal for a *bare* slice
  scalar (`layout.rs:271`, pinned by `scalar_size_align_refuses_a_slice` —
  `layout.rs:1214-1220`; the pin's body calls the `scalar_size_align` wrapper,
  not `_ww` directly, round-3 review P2-5) is retained **unchanged** — REQ-1
  routes slice fields through `LayoutBuilder::size_align` instead, never
  through this function. **One existing pin is deliberately inverted, not
  broken (round-3 review P1-3):** `member_ty_refuses_a_slice`
  (`src/backend/qbe.rs:2736-2743`, the file's only `#[should_panic]`) asserts
  exactly the refusal REQ-2 replaces — it is **retired/inverted** in the same
  commit (member_ty now spells `:{SLICE_TYPE_SYMBOL}`), with its doc comment's
  "emit relies on that ban" rationale replaced by REQ-2's ordering argument;
  NFR-4's promise covers it via this explicit retirement, so no implement
  worker meets an unexplained red test.

## Golden tests (phase-exit criteria)

Phase 1 goldens (layout + backend + all bans + declared-aggregate relaxation):

- **G1 (declared struct field, shared `Slice[T]` only, inline-only per Ruling B).**
  Full fixture and expected stdout:

  ```sooth
  import: intrinsics * ;
  import: hosted::show | . | ;
  import: core::prelude * ;
  import: core::combinators c | times | ;

  type: Window view Slice[i64] lo usize ;

  : total inline ( Window -- i64 )
    |w|
    &w &view @ | s |
    0 s len >i64 ~[ |i| s i >usize &> @ add ] times
  ;

  : main ( -- )
    0 5 fill | a |
    &a slice 0 >usize Window | w |
    w total .
    a drop
  ;
  ```

  Expected stdout: `0` (the sum of a 5-element zero-filled array). `total` and the
  driver are the exact hand-written `times`/`&>` loop (no `Iterator` impl is used
  or implied; the loop body is copied verbatim from the working `sum` in
  `examples/slices.sth`). Verified 260907: the fixture's **declaration** reaches
  exactly the expected pre-capability rejection today (`field \`view\` of type
  \`Window\` has type \`Slice[i64]\`) — note the declaration check fires before
  word bodies are checked, so that run proves nothing about the body; the body's
  syntax and word resolution were verified separately (with the field typed
  `i64` to pass the declaration gate, `&w &view @` parses, resolves, and
  type-checks through to `len`). **Enum twin**
  (parseable form, per contract review's P1-7 fix — an unnamed concrete payload
  does not parse):`type: Cell | Empty | Full v Slice[i64] ;`, constructed,
  matched, and its payload projected and dropped. **This twin asserts
  *build-and-run* only** (round-5 review P0-3): the variant path's borrow
  tracking is witnessed separately by **G-alias-iv**, because a passing build
  proves nothing about whether the arm-bound payload is tracked. **One residual (round-2
  review, G1 note):** `total` takes `Window` **by value** and never `drop`s
  it — correct as written, not an omission: `Window` is `Copy` under Ruling A
  (a shared-slice-only container with a plain `usize` field), so it carries no
  drop obligation; a worker must not add a spurious `drop`.
- **G1-input (NEW — Ruling B; unlocated, round-2 review NEW-4/P2-J).** A
  `Window`-typed value passed as an input to a **non-inline** word is rejected
  by the existing `word_entry.rs:172-179` input arm, now reached over an
  aggregate — an **unlocated, type-naming** error (REQ-6), the same class as
  G2/G5, not a located one (correcting the round-2 draft's "located error"
  description).
- **G1-mut-hard-ban (NEW — Ruling A).** `type: MutHolder val !Slice[i64] ;` stays a
  hard reject, unchanged text (`stored_reference_error`, verified 260907: today's
  message for a `!Slice[i64]` field, unaffected by this spec).
- **G1-nested (NEW — REQ-5).** `type: Outer h Holder ;` (nesting `Holder`, itself
  holding a shared slice) is admitted and tainted, not rejected.
- **G2 (non-inline output rejected — taint, unlocated, type-naming, unchanged
  text).** A non-inline `( -- Window )`-style word returning a slice-bearing
  aggregate is rejected with `stored_reference_output_error`'s existing,
  unlocated, type-naming text (byte-identical to G5's message).
- **G3 (capture rejected — taint, two twins, Ruling D's fence).** (i) A
  quotation capturing a slice-bearing local at an **escaping** boundary is a
  located capture error (the pre-existing `escaping` check, unconditional on
  linearity, so no change here). (ii) **Owning-closure twin (soundness P0-2's
  original motivation, now closed by the broader fence rather than a
  linear-only exclusion):** the same capture at an `owning [ ... ]` boundary
  is also rejected — under Ruling D's fence (REQ-4b), *every* slice-bearing
  capture is rejected regardless of `owning`/`escaping`, so this twin is
  witnessed by the same mechanism as (i) and as G-capture-fence below, not by
  a separate `owning && linear` exclusion.
- **G-capture-fence (NEW — Ruling D; fixture completed per round-3 review P2-1).**
  The exact reproduced fixture, imports included:

  ```sooth
  import: intrinsics * ;
  import: hosted::show | . | ;
  import: core::prelude * ;

  : use ( [ -- ] -- ) call ;

  : main ( -- )
    0 4 fill | a |
    &a slice | v |
    [ v len >i64 . ] use
    a drop
  ;
  ```

  A bare `Slice[i64]` local captured into a materialized closure at a
  **plain, non-escaping, in-frame** boundary (neither G3 twin's shape) is a
  located error. This is the fixture that ICEs at `qbe.rs:526` today (verified
  260908); the golden pins the fence, not the ICE.
- **G-alias-ii (NEW — Ruling C / REQ-4d write side; restated over a shared
  slice per round-2 P0-C, re-channeled per round-3 P0-1/P0-2).** A **shared**
  slice packed into a `Holder`; a second `&!` borrow of the root array is
  taken while the `Holder` is still live (read afterward — last-use liveness,
  `engine.rs:767-771`, is what keeps G1's own drop-after-last-use legal, so
  the `Holder` must have a post-borrow use) — **rejected**. This is the
  write-side witness: with REQ-4d's deriv propagation onto the constructed
  slot, the second `&!` trips the **borrow-side exclusivity scan**
  (`word_families.rs:243`, `live_deriv` → `conflicting_borrow_error`; the
  predicate `d.owned_root.as_deref() == Some(rest)` matches the deriv REQ-4d
  propagated onto the `Holder` binding, visible via `live_derivs`'s
  `scope.bound` arm, `engine.rs:963-968`); `aliasing_origin` →
  `aliased_place_borrow_error` (`:268-271`) is the alias-keyed sibling, and the widened
  `conflicts` closure (`word_families.rs:487`, `mutable || other_mutable`)
  backstops the projection path. (Round-4 review P1: an earlier draft
  attributed this to `live_mutable_borrow_of` → `terms.rs:283`, but that
  guard fires only when *naming a local* while a mutable borrow lives — the
  opposite order to this golden.) (The round-2 draft's "consume the source array
  after packing" spelling is **deleted**: probed 260908, `0 4 fill |a| &a
  slice |s| a drop s len >i64 .` builds and prints `4` — consuming a `Copy`
  root while a view is live is legal today and stays legal, so no golden may
  pin it; see REQ-4d's scope note.)
- **G-alias-iii (NEW — Ruling C / REQ-4d read side, round-2 review P0-B;
  re-spelled per round-3 P0-2).** A slice is projected out of a `Holder`
  through `@` (G1's own idiom, `&w &view @ |s|`), and **while that projection
  is live** a second `&!` borrow of the root is taken — **rejected**. This is
  the read-side witness: without `@`'s deriv/alias forwarding, the fetched `s`
  is invisible to every guard and the golden would pass as an inert accept.
  (The round-2 draft's "consume the root after the projection" spelling is
  deleted for the same probed Copy-root reason as G-alias-ii's.)
- **G-distinct-root (NEW — Ruling F, round-4 review P1).** A struct with two
  slice fields (`type: Pair a Slice[i64] b Slice[i64] ;`) constructed from
  views of **two different arrays** is a located error (distinct
  `owned_root`s, REQ-4(d)'s multi-operand rule); the same type built from two
  views of **one** array builds and runs — pinning both the ban and its
  boundary.
- **G-poly-launder (NEW — round-5 review P0-1 / REQ-4d site 4).** Two twins,
  because the hole predates this spec:
  (i) **today-red regression twin, bare slice, no new capability needed** — the
  exact `thru` program in REQ-4(d) site 4 builds and prints `99` at `486eda4`
  and must become a rejection ("conflicts with a live borrow of `a`", the same
  diagnostic the `thru`-free program already produces);
  (ii) **aggregate twin** — the same shape with `&a slice 0 >usize Window`
  passed through `thru` in place of the bare slice, rejected at the second
  `&!a`. A worker must keep twin (i): it is the one golden in this spec that
  fails on an unmodified tree, so it is the proof that site 4's forward is
  load-bearing rather than decorative.
- **G-store-escape (NEW — round-5 review P0-2 / REQ-4d site 5 rule (i)).** The
  cross-frame stash: a **non-inline** `: stash ( &!Window -- )` that builds an
  array in its own frame, slices it, and stores the view into the caller's
  `Window` through the `&!` input. Rejected with site 5's own one-line
  message. The `i64`-field analogue of this program builds and runs today
  (verified 260909, caller reads `7`), so the golden pins a genuinely new
  rejection on a reachable path, not a hypothetical one.
- **G-store-distinct-root (NEW — round-5 review P0-2 / REQ-4d site 5 rule
  (ii), Ruling F).** In-frame: a `Window` constructed over root `b`, then
  `&!w &!view  &a slice  !` storing a view of a *different* array into it —
  rejected (distinct `owned_root`s). The same store from a view of `b`
  (the root the `Window` already carries) is admitted, pinning the boundary
  rather than a blanket ban on field stores.
- **G-alias-iv (NEW — round-5 review P0-3 / REQ-4d site 6 + the `Variant`
  taint arm).** A shared slice packed into an enum variant
  (`type: Cell | Empty | Full v Slice[i64] ;`), eliminated through an owning
  arm (`~[ ( Full ) &v @ swap drop ]`), and a second `&!` of the root array
  taken while the extracted slice is live — **rejected**. This is the variant
  path's conflict witness, and it fails without either half of the fix (the
  `Type::Variant` taint arm or site 6's deriv forward), which is why G1's enum
  twin cannot stand in for it.
- **G-sweep-array (NEW — REQ-4c).** A slice-bearing type as an interned array
  element is rejected (`declarations.rs:1117`, unchanged).
- **G-sweep-cell (NEW — REQ-4c).** A slice-bearing type as an interned cell payload
  is rejected (`declarations.rs:1126`, unchanged — the only heap-storage guard).
- **G-sweep-slice-element (NEW — REQ-4c; witness fixed, Ruling E).** A
  reference-shaped slice element is rejected (`declarations.rs:1175`,
  unchanged) — witnessed directly over `Slice[&i64]` (a parseable, interned
  spelling, `parser.rs:7299-7302`/`declarations.rs:1160`), not the round-2
  draft's unconstructible "`Holder` holding a `&T`" (REQ-5 keeps a `&T` field
  hard-banned, so that `Holder` can never be declared).
- **G-sweep-fill (NEW — REQ-4c).** `fill` refuses to construct an array of a
  slice-bearing element type (`check.rs:478`, unchanged).
- **G-sweep-cellctor (NEW — REQ-4c).** `^` refuses to construct a cell over a
  slice-bearing payload (`word_families.rs:1080`, unchanged).
- **G-quot-effect-output (NEW — REQ-4c).** A materialized quotation declaring a
  slice-bearing output is rejected (`captures.rs:516`, unchanged).
- **G-quot-effect-input (NEW — REQ-4c's addendum).** A materialized quotation
  declaring a slice-bearing **input** is rejected by the new input arm on
  `check_quotation_reference_free_effect`.
- **G-poly-twin (NEW — REQ-4a; witness fixed, round-2 review NEW-2/P0-D).** A
  poly word **declaring a concrete** slice-bearing output (e.g. `: w ( 'T --
  Window )`) is rejected (`contains_poly_reference`, `audits.rs:360`'s
  `PolyType::Concrete` arm) — not a generic output slot *later instantiated*
  at a slice-bearing type: a declared `PolyType::Var` output returns `false`
  unconditionally (`audits.rs:366`), and no per-instantiation audit exists to
  reject that shape (a named gap, Deferred).
- **G6 (IR pin).** An `emit_ssa_with_manifest`-captured pin (`src/driver.rs:897`)
  that the slice field lays out at the two-word offsets and drop is a no-op over
  the slot. Uses a **mixed** linear+slice struct (a slice field alongside an
  owning-cell field) so a mutation deleting `field_is_linear`'s slice handling
  cannot survive undetected by the struct's own linearity flag.

Phase 2 goldens (return-bundle ABI/plumbing only):

- **G4 (inline >=2-output slice word).** Full fixture and driver:

  ```sooth
  import: intrinsics * ;
  import: hosted::show | . | ;
  import: core::prelude * ;

  : two-out inline ( Slice[i64] -- i64 Slice[i64] )
    |s| 0 s
  ;

  : main ( -- )
    0 3 fill | a |
    &a slice two-out | x r |
    x .
    a drop
  ;
  ```

  Expected stdout: `0`. Verified 260907: this exact fixture panics at
  `layout.rs:271:29` on tip (`internal error: entered unreachable code: a slice
  value resolves via \`slice_layout\`, not a scalar`) — the panic this phase's
  work removes.
- **G5 (non-inline twin — unlocated, unchanged text, same message as G2).** The
  same body without `inline` is rejected by `check_reference_free_signature`,
  byte-identical to G2's message (REQ-6).
- **G-poly-bundle (NEW).** A polymorphic word instantiated with a slice-bearing
  `out_arity >= 2` resolved output builds and runs, exercising the
  per-instantiation bundle sites (`check.rs:1111`, `:1125`).

Unit tests sit beside each changed stage function, happy path plus one
rejection each, named `thing_condition_expected`:

- `layout.rs` sizer/projection; `qbe.rs`
  member/load/store/`module_has_slice`; `declarations.rs::check_no_stored_references`;
  `word_entry.rs::check_reference_free_signature`.
- `ir/func_builder/word_families.rs`'s `store_field`/`slot_value` aggregate-arm
  slice routing (round-4 contract review P1-1).
- REQ-4(d)'s six propagation sites, one test each: `check/terms.rs`'s
  output-push deriv+alias forwarding and the distinct-root rejection (site 1,
  `terms.rs:1194-1198`, Ruling F); `word_families.rs`'s `@` fetch-arm
  forwarding (site 2, `:583-587`); `check/engine.rs`'s `prov.borrow` deriv
  inheritance (site 3, `:364-380`); the four dispatch output pushes (site 4,
  `poly.rs:7955-7957`/`:7994-7996`/`:2090-2092`/`:2614-2616`); the `!`/`+!`
  store's two rules (site 5, in `terms.rs:449-510`); and the
  anonymous-receiver projection arm's deriv forward (site 6,
  `word_families.rs:444-447`).
- `check.rs`'s `dup` alias retention for a reference-bearing aggregate
  (`:3266`).
- `builtins.rs::contains_reference`'s new `Type::Variant` arm, and
  `captures.rs::classify_capture`'s `Type::Variant` aggregate arm.
- `word_families.rs`'s `conflicts`/`consumed_place_conflict` widening;
  `captures.rs`'s `check_capture_admission` (the materialization fence) and
  `check_quotation_reference_free_effect`; `audits.rs::contains_poly_reference`.
- A guard test beside `back_edge_outs_forwards_surviving_set_along_index_map`
  (`terms.rs:3055`) asserting `check_reference_across_back_edge` rejects a
  deriv-carrying argument, per the Deferred back-edge note.

## Phases

**Phase 1 — declared reference-bearing aggregates: layout, backend, every ban,
declared-aggregate relaxation, landed together.** This is the soundness-critical
phase; nothing here is deferrable to Phase 2 (soundness review P1-3: the layout
gate `LayoutBuilder::size_align` is global — it serves declared structs, the
`intern_output_bundles` return bundle, and a closure-env struct alike — so the
moment REQ-1 answers it, every synthesized-aggregate channel is open too; the
bans must land in the same commit as the layout work, not one phase later).
Concretely:

- REQ-1: `LayoutBuilder::size_align` gains the `Type::Slice` arm;
    `scalar_size_align_ww`'s refusal is retained unchanged.
- REQ-2: `qbe.rs`'s `member_ty`, `field_load_op`/`field_store_op`,
    `Instr::Load`/`Instr::Store` selection, and `module_has_slice` all gain their
    named slice handling; construction/projection blit, not scalar-load/-store.
- REQ-4(a/b/c/d): the non-inline output/input ban (already active, re-verified
    over aggregates plus its poly twin, corrected witness), the Ruling D
    materialization fence (bare slices and slice-bearing aggregates alike, at
    every boundary), the seven-site closed-enumeration re-verification (fixed
    witness on the slice-element gate), and the new alias-set propagation —
    write side (`terms.rs`'s output push), read side (`@`'s fetch arm), and both
    conflict guards — **all active before or in the same commit as** REQ-5's
    relaxation.
- REQ-5: `check_no_stored_references`'s two named sweeps relax to
    admit-and-taint for shared-`Slice[T]`-bearing fields only (corrected
    predicate); the recursive shape stays deferred (Ruling E) and the linearity
    note is recorded.
- REQ-3's return-bundle *layout* work is unavoidably exercised by this phase too
    (see above), but its ABI/codegen plumbing (making the bundle actually usable
    end-to-end for a multi-output word) is Phase 2's job — Phase 1 only needs the
    layout gate to answer, not the calling convention around it, to keep its own
    goldens (G1 and its variants, G2, G3, G-alias-*, G-sweep-*, G6) green.
  **The Ruling D fence lands in this same commit as REQ-2's `field_load_op`/
    `field_store_op` blit changes (NFR-3)**: without it, the blit change turns
    today's closure-capture ICE (`qbe.rs:526`) into silent env-slot corruption,
    not merely a missed capability.

  Goldens: G1 (+ enum twin, G1-input, G1-mut-hard-ban, G1-nested), G2, G3
  (+ owning twin), G-capture-fence, G-alias-ii (write side), G-alias-iii (read
  side), G-alias-iv (enum elimination, site 6 + the `Variant` taint arm),
  G-distinct-root (Ruling F), G-poly-launder (site 4, both twins),
  G-store-escape and G-store-distinct-root (site 5), all G-sweep-* goldens,
  G-quot-effect-output, G-quot-effect-input, G-poly-twin, G6. Unit tests beside
  every changed function listed above. This ordering is load-bearing (NFR-3):
  the taint bans and the capture fence gate every new construction the layout
  admits, in the same commit. Effort **L**, difficulty
  **hard** (real IR-layout and backend work, the closed enumeration of guard
  sites in REQ-4(c) to re-verify, plus a soundness-critical in-frame
  alias-tracking extension (Ruling C: `Deriv`-primary borrow propagation) and
  the closure-env capture fence).

**Phase 2 — synthesized-aggregate return-bundle ABI (re-scoped, smaller than the
first draft).** The layout gate is already answered by Phase 1 (see above), so
this phase is **not** layout work: it is the struct-return calling-convention
plumbing that makes an inline `>= 2`-output word with a slice output actually
build and run end-to-end (G4), including the polymorphic and splice-record bundle
sites (`check.rs:1111`/`:1125`, G-poly-bundle), plus re-witnessing that the
non-inline twin (G5) and a capturing closure (already covered by Phase 1's REQ-4b)
stay rejected. Effort **M** (downsized from the first draft's "M-L": the
soundness-critical layout and bans landed in Phase 1; this phase is ABI plumbing
over an already-answered layout gate), difficulty **hard**. Re-run growth signals
(CLAUDE.md) on `layout.rs`, `check.rs`, and `qbe.rs` at phase exit.

**Phase 3 — evidence, roadmap, growth re-check.** IR pins finalized; the
S6d-PREREQ roadmap entry updated (capability shipped, what remains for S6d: the
DQ4 grounding route and the deferred poly-body fix below) — **verify-and-finish
only** (round-3 review P2-2; commit-anchored per round-4 review P0-1): the
entry fixes below are **already applied and committed alongside this spec**
(Rulings A–F recorded, wall-1 re-stated to the ruled
`LayoutBuilder::size_align` route, ICE attribution corrected to a capture-fence
matter, `builtins.rs:578` fixed, "evidence-scoped, no spec written" replaced by
the spec link, `ROADMAP.md`'s P7b row naming S6d-PREREQ); a Phase-3 worker
**verifies these landed on the implementation base** and does not re-edit
them, then adds only the
capability-shipped sentence and the growth re-check on every file this slice
grew (`layout.rs`, `check.rs`,
`check/declarations.rs`, `check/word_entry.rs`, `check/word_families.rs`,
`check/captures.rs`, `check/audits.rs`, `check/terms.rs`, `ir/func_builder/`,
`backend/qbe.rs`).
Effort **S**, difficulty **standard**.

### Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Declared reference-bearing aggregates: two-word slice-slot IR layout via LayoutBuilder::size_align with frontend blit routing in ir/func_builder store_field/slot_value, backend member/load/store/module_has_slice refusals retained, the REQ-4 escape bans (non-inline output/input incl. poly twin, Ruling D's unconditional materialization-boundary capture fence for bare and aggregate slices, closed-enumeration guard re-verification, six-site in-frame deriv+alias propagation (construction push, @ fetch, prov.borrow inheritance, poly-call/member-dispatch output pushes, the !/+! field store with its out-of-frame store error, the anonymous-receiver projection arm) plus both conflict-guard treatments, the Type::Variant taint-predicate widening with its consumer-inventory walk, dup's alias retention for reference-bearing aggregates, and Ruling F's distinct-root rejection at construction or field store) landed in the same commit as the check_no_stored_references admit-and-taint relaxation for shared Slice[T] fields only (corrected predicate); the recursive Slice[Holder] shape stays deferred (Ruling E)",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Synthesized-aggregate return-bundle ABI/codegen plumbing (monomorphic, polymorphic, and splice-record bundle sites) over the layout gate Phase 1 already answers; inline >=2-output slice word builds and runs, non-inline twin and capturing closure stay located errors",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Evidence, roadmap S6d-PREREQ entry update and spec link, growth-structure re-check on every grown file",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
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
- **`!Slice[T]` as a declared field.** Ruling A defers this; it flows only through
  synthesized return bundles (REQ-3), where positional unpack sidesteps `@`'s
  `is_copy` gate entirely.
- **A readback story for a non-`Copy` referent behind `@`.** `@`'s `is_copy` gate
  (`word_families.rs:564-567`) is unchanged; no new move-out-of-a-field mechanism
  is introduced.
- **Closure-env slot widening for a captured slice (Ruling D).** Making
  `build_env`'s one-word-per-capture encoding (`quotation.rs:175-192`)
  support a two-word (or wider) captured value is moot under the Ruling D
  fence: this spec bans every slice-bearing capture at every materialization
  boundary outright, so there is nothing left needing a wider env slot. If a
  future slice ever needs to capture a slice into a closure, this is the
  mechanism it would have to build (with `bind_env_capture`,
  `ir/func_builder/mod.rs`).
- **The instantiation-time poly gap (round-3 review P1-3; its reachability
  argument was WRONG and is retracted — round-5 review P0-1).** A poly word's
  declared effect is empty
  (`word_entry.rs:53-55`) and `audit_poly_reference_free_signature` runs once
  over the declared signature, where a generic slot is `PolyType::Var(_) =>
  false` (`audits.rs:366`); there is no per-instantiation audit (sole
  production caller `audits.rs:289`). So once REQ-5 makes `Window`
  declarable, a non-inline poly word `( 'T -- 'T )` instantiated at `Window`
  passes both of Ruling B's arms.
  **The round-3/round-4 text then argued this was not exploitable, on the
  ground that a pass-through only returns the value to the frame that built
  it. That argument is about lifetime and is silent about exclusivity, which
  is the property that actually breaks:** the pass-through *is* the exploit,
  it needs no aggregate, and it runs today (the `thru` program in REQ-4(d)
  site 4 prints `99` at `486eda4`). **The site is therefore no longer
  deferred**: it moves into REQ-4(d) as site 4, and its bare-slice form is a
  live pre-existing bug fixed by the same one-line-per-push forward in Phase
  1. What remains deferred is only the **signature audit**: a per-instantiation
  `audit_poly_reference_free_signature` that would reject a non-inline poly
  word instantiated at a slice-bearing type outright, rather than relying on
  propagation to make its misuse a located error. That is its own slice; S6d
  does not need it (its consumers are monomorphic or inline).
- **Self-tail loops over a tainted aggregate (round-5 review P2).** Two notes
  from the round-5 back-edge probe. (1) A stated capability boundary: once
  REQ-4d propagates a deriv onto a `Window`, `check_reference_across_back_edge`
  (`check.rs:1670-1693`) rejects any argument whose deriv has a non-static
  `owned_root`, so a tainted `Window` can **never** cross a self-tail back
  edge. That is safe (the guard is the mask, see (2)) but real: S6d's iterator
  consumers are loop-shaped, and driving one over a `Window` in a self-tail
  loop will be rejected until a loop-aware borrow story exists. This spec does
  not change the guard. (2) The mask is load-bearing: `back_edge_outs`
  (`terms.rs:2815`) forwards only `surviving` along the index map and drops
  `deriv`, a latent twin of the poly-push laundering (site 4) that is
  unreachable only because the guard rejects first. If that guard is ever
  narrowed, the hole opens with no test watching, so Phase 1 adds a guard
  test beside `back_edge_outs_forwards_surviving_set_along_index_map`
  (`terms.rs:3055`) asserting the rejection, not the forward (see the
  unit-test list).
- **Multi-root `Deriv` (Ruling F's lifting condition).** `Slot.deriv` is a
  single `Option<DerivId>` (`engine.rs:515`) and `Deriv.owned_root` a single
  `String` (`engine.rs:56-83`); until `Deriv` can represent several roots,
  REQ-4(d)'s multi-operand rule (Ruling F) rejects constructions whose
  slice-bearing operands view different arrays. A multi-root `Deriv` (or an
  aggregate-keyed deriv registry) is a borrow-checker design slice of its own
  — it is what would let a `Pair` of windows over two buffers be tracked
  soundly.
- **The recursive shape `type: Holder val Slice[Holder] ;` (Ruling E).**
  `check_recursion` already passes it and `check_no_stored_references`'s
  relaxed struct sweep would admit `Holder` as a field type, but
  `check_slice_element_gate` (REQ-4c, unchanged) still hard-rejects
  `Slice[Holder]` as a slice element, since `Holder` is reference-bearing. This
  spec does not relax the element gate, so the shape stays undeclarable; a
  future slice that wants it must rule on relaxing `check_slice_element_gate`
  itself, which is a separate, unexamined question (does an
  admit-and-taint-shaped element predicate mirroring REQ-5's corrected one
  apply here too, and does that reopen any escape the field-position relaxation
  does not).
