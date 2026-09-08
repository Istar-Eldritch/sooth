# P7b.S6d brief — slices as Iterator targets (the built-in-view lift)

- Date: 2026-09-07. Base: `a9eca84` (worktree `p7b-s6d`, an ancestor of `main`
  at `7404a71`; nothing to rebase).
- Recorded in the roadmap's own S6d entry (`P7b-higher-kinded-types.md`,
  corrected 260907 after a live probe — two earlier sketches in the entry's
  history were wrong, see that entry's opening line).
- Source: probe round **S6d-1, run 260907** — verbatim log at
  [slice6d-probes](./slice6d-probes.md) (one `prober` worker, S6d-1..S6d-6).
  The round confirms the roadmap entry's own claims exactly; nothing here
  overturns them, it only adds the evidence trail and the mechanism-level
  detail the roadmap entry left implicit.

## Problem

`Slice['T]`/`!Slice['T]` have existed since P7.S3c as runtime-length views:
`Type::Slice`, interned per `(element, mutable)` with no count (the length is
a runtime component of the value, not a monomorph parameter), `slice`/
`subslice`/`len` words, one instantiation over all lengths. `impl: Iterator
for Slice[i64]` and `!Slice[i64]` parse as impl targets, but `next`'s
App-headed member row (`'It['T] -- Step['T 'It['T]]`) hits the S2-6 concrete-
target fence — confirmed byte-for-byte in S6d-1/S6d-2 of the probe log:

```text
error: trait member `next` of `Iterator` (line 5, col 5) applies the trait variable `'It`, but the impl target `Slice[i64]` is concrete
  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead
```

identical for `!Slice[i64]`. The generic spelling `Slice['T]` is not a
workaround — it fails earlier and is unreachable as an impl-target spelling
at all (S6d-3): `error: unknown type 'T at line 4, col 26`. Nothing in the
parser's target grammar takes a bare type variable as a slice's element.

**Why the P7b.S8 Range lift does not transfer.** S8 admitted
`impl: Iterator for Range[i64]` by having `impl_target_pattern_poly_type`
(`src/parser.rs:4242`) reverse-lookup the struct/enum instantiation registry
and re-wrap a folded `Concrete(Type::Struct | Type::Enum)` back into
`PolyType::Generic { is_enum, idx, module, .. }`, which
`ImplTarget::is_mono_ctor_app` (`src/ast.rs:2569`) then recognizes, exempting
it from the concrete-target fence. That path is keyed on `GenericId`'s
`(is_enum, idx, module)` triple into `GenericStructDecl`/`GenericEnumDecl` —
registries `Type::Slice` was never minted through (it comes from
`intern_slice_type`, keyed structurally on `(element, mutable)`, no header).
This is exactly the roadmap's own S6-carve-out ruling restated for S6d: a
third `GenericId` case needs a new registry and instantiation-pair bridging,
not a widened match arm. `is_mono_ctor_app` cannot be made true for a
`Type::Slice` target without inventing a shape it was never designed to
carry.

Everything else needed to write `next`'s body already works (S6d-4/5/6):

- `Slice[i64]` (shared) is `Copy`, `!Slice[i64]` (mutable) is not
  (`src/check/builtins.rs:520`, pinned since P7.S3c by
  `is_copy_shared_slice_is_copy_mutable_slice_is_not`) — a shared iteration
  can freely `dup` its view; a mutable one is a real linear consume, exactly
  the shape `next`'s `'It['T]` consumption already wants.
- `len ( Slice[T] -- usize )` and `subslice ( Slice[T] usize usize -- Slice[T]
  )` both consume the receiver and both preserve the receiver's mutability
  into their output (`word_families.rs:1020-1038`, `:842-861`) — a `!Slice`
  `next` yields a `!Slice` remainder, a `Slice` `next` yields a `Slice`
  remainder, with no cross-mutability case to design for.
- `&>`/`&!>` (`word_families.rs:32-63`) read one element, bounds-checked
  against the view's *runtime* length — the `More` arm's payload.
- The dispatch matcher's plain `Concrete` equality arm
  (`match_impl_target_rec`, `src/check/poly.rs:9276-9278`) already matches
  `Type::Slice` values correctly; dispatch is not the blocker, only
  declaration-time grounding is (S6d-1/2 fire before dispatch is ever
  exercised).

## Discovery questions (per the roadmap entry, restated with the probe evidence)

- **DQ1 — shared vs exclusive iteration.** Does the impl land for `Slice[i64]`
  (shared), `!Slice[i64]` (exclusive), or both? S6d-4 shows nothing forces a
  choice: `Copy`-ness only changes whether the *caller* can `dup` the view
  before handing it to `next`, not whether `next` itself can be written — the
  member body consumes its receiver either way (S6d-5's `len`/`subslice`
  both consume). Both targets are independent `Concrete` types today (two
  separate `impl:` blocks, same as `List`/`Range[i64]` are independent
  targets in S8), so "both" costs nothing beyond writing both bodies once
  the fence lifts.
- **DQ2 — remainder mutability.** Answered by S6d-5, not open: `subslice`
  preserves the receiver's mutability, so a `Slice[i64]` impl's `More` arm
  carries a `Slice[i64]` remainder and a `!Slice[i64]` impl's carries a
  `!Slice[i64]` remainder. No mixed-mutability shape exists to rule on.
- **DQ3 — the linear-discipline story for a view that owns nothing.**
  Narrower than it first looked: `Step::Done`/exhausted-view accounting
  needs no new mechanism (a slice's `drop` is already a no-op; the
  Step-row protocol's "no live value to drop" shape covers it). The
  spike (round S6d-2) found the *real* shape-level problem living here
  instead — see DQ5.
- **DQ4 — how does the fence actually get lifted for a
  `Concrete(Type::Slice(..))` target?** **Spiked and answered, round
  S6d-2 (260907).** Candidate (b) (ground before fencing) is a confirmed
  dead end — `ground_member_type` takes a `Type`, not a `PolyType`, and
  structurally cannot carry a member-local substitution no matter when it
  runs. Candidate (a) (a dedicated slice-grounding path) works: a
  sentinel-substitution trick grounds `impl: Iterator for Slice[i64]`
  correctly with a small, verified diff (one new function, one new
  branch, no registry threading), at the cost of a documented, narrow
  invariant exception on `PolyType::Generic` — or a bigger, cleaner
  `PolyType::SliceApp` variant if that exception is ruled unacceptable.
  Full evidence: [slice6d-probes §Round S6d-2](./slice6d-probes.md#round-s6d-2-dq4-spike-run-260907).
- **DQ5 — `Step`'s `Rest` field cannot hold a slice at all today,
  independently of DQ4.** **Spiked and closed on option (i), round S6d-3
  (260907).** `check_no_stored_references` (`check/declarations.rs:1081`)
  rejects `Step[i64 Slice[i64]]` because `Type::Slice` is reference-shaped
  by design (same rule as `&T`) — and this is not a technicality to route
  around: the rule's own rationale (`declarations.rs:1146`, "a reference
  ... may not outlive [the local it borrows], so it cannot be put anywhere
  that survives the borrow") is the *entire* escape-safety mechanism, since
  Sooth has no lifetime-tracking pass at all. A carve-out for `Type::Slice`
  removes the only thing preventing a `Step` value from outliving the
  buffer its slice views — unsound by the rule's own stated argument, not
  merely untested. Independently, it also doesn't build: a spiked 1-line
  carve-out (`contains_reference`'s `Type::Slice(..) => true` → `false`)
  compiles the `impl:` but panics at IR-lowering
  (`layout.rs:271`, `scalar_size_align_refuses_a_slice`'s deliberate
  refusal — enum/struct payload layout has no registry path for a
  two-word slice-shaped field at all). **Option (i) is closed on both
  soundness and buildability.** Full evidence:
  [slice6d-probes §Round S6d-3](./slice6d-probes.md#round-s6d-3-dq5-spike-run-260907).
  **Option (ii) closed too, round S6d-4 (260907):** the Option-row shape
  (`next ( 'It['T] -- Option['T] 'It['T] )`, remainder as a bare stack
  value) parses and checks with the sentinel fix (which needs a second
  half: `resolve_mono_member_call`'s `lifted_mono` condition), but dies at
  lowering on the **same** `layout.rs` slice-field refusal — any ≥2-output
  word with a slice output gets a synthesized return-bundle struct
  (`intern_output_bundles`), and that struct is the enum-payload wall
  under another name. The dodge was cosmetic. Two further pre-existing
  walls surfaced: `check_reference_free_signature` bans a *non-inline*
  word from declaring a slice output at all, and the poly-body stack
  checker loses an App-headed dispatch call's outputs, so the generic
  consumer fails regardless. Full evidence:
  [slice6d-probes §Round S6d-4](./slice6d-probes.md#round-s6d-4-dq5-option-ii-verification-run-260907).
  Under **any** protocol whose `next` has ≥2 outputs, slices-as-Iterator
  is gated on one and the same missing capability: **slice-shaped fields
  in synthesized and declared aggregates** (IR layout) plus a
  checker-level storage-class/taint story for reference-bearing aggregates
  (the escape-safety rationale applies to the aggregate exactly as to the
  bare slice).

**DQ4 spiked, round S6d-2 (260907) — [full log](./slice6d-probes.md#round-s6d-2-dq4-spike-run-260907).**
Candidate (b) is a confirmed dead end: `ground_member_type` takes a plain
`Type`, not a `PolyType`, so it structurally cannot carry a member-local
substitution regardless of ordering — the fence isn't premature, the
function it guards can't express what an App-headed row needs. Candidate
(a) works and is *smaller* than expected: a sentinel-substitution trick
(fake `PolyType::Generic` carrying the slice's element, rewritten back to
`Concrete(Type::Slice(..))` before it reaches the code that would
dereference its bogus header) grounds `impl: Iterator for Slice[i64]`
cleanly with one new ~25-line function and one new ~40-line branch, no
changes to `MutRegistries`'s 27 construction sites. The cost is a
momentary, documented violation of `PolyType::Generic`'s own invariant
(narrow and containable, per the spike) — or, if that's ruled
unacceptable, a dedicated `PolyType::SliceApp` variant touching every
exhaustive `PolyType` match (bigger, unverified, more honest to the
invariant).

**DQ4 is answered, but it was not the whole blocker.** The spike surfaced a
fourth, independent problem: even with candidate (a) grounding correctly,
the resulting `Step[i64 Slice[i64]]` monomorph is rejected by
`check_no_stored_references` — `Type::Slice` is reference-shaped by design
(same rule as `&T`), and `Step['T 'Rest]`'s `Rest` field cannot hold a
reference-shaped payload today. This has nothing to do with App-headed
rows or grounding mechanics; it's the ordinary field audit, run once
against the final monomorphic type, and it fires no matter how grounding
is reached. See DQ5 below.

## Scope (unchanged from the roadmap entry; not yet a spec)

Per `P7b-higher-kinded-types.md`'s S6d entry: admit element-concrete slice
targets (`Slice[i64]`, `!Slice[i64]`) to an Iterator-target route (mono,
like Range's, once DQ4 is resolved); library impl over existing words
(`len`, `&>`/`&!>`, `subslice`, `Step` construction) — the S6 construction
wall is not in play (no `Cons`-style self-reference reconstruction; `Step`
is a plain-field enum ctor). `Slice['T]` (generic element) is out of scope —
it is unreachable as a target spelling (S6d-3), not merely deferred; making
it reachable would be new parser-grammar work the roadmap entry never asked
for. Size: `S-M` per the roadmap entry (checker/target-grammar only, no
`src/ir/` change).

## Explicitly out of scope

- `Slice['T]` (generic-element) targets — unreachable spelling, not a
  discovery question (S6d-3).
- Any change to `array`'s own S6-carved-out constructor gap — a different,
  already-recorded wall (array has no `Type::CtorImage` to dissolve into;
  slices are a different built-in shape entirely).
- `map`/`append`-style per-impl members over slices — S8's own `for_each`/
  `fold` ruling (R4) applies here too until a future slice revisits it.
- Fusion/performance measurement for slice iteration — not asked for by the
  roadmap entry.

## Status

Blocker precisely located and evidenced (probe round S6d-1). DQ4 (the
fence-lift mechanism) is spiked and answered (round S6d-2): candidate (a)
works, candidate (b) is dead. DQ5 (round S6d-3) is spiked and **partially
ruled**: the carve-out option (i) is closed — unsound by
`check_no_stored_references`'s own rationale (no lifetime tracker exists;
the carve-out removes the only escape-safety mechanism) and independently
unbuildable today (deliberate `layout.rs` refusal for slice-shaped enum
payload fields). What remains open is a genuine design choice, not a
mechanism spike: **(ii) design a non-Step exhausted-case shape for slice
iteration** (cheaper per the evidence, but the shape itself needs
designing — e.g. does `next` return the element and remainder as two
separate stack values with a boolean/sentinel instead of packing them into
an enum, and if so does that split the `Iterator` trait's protocol
type-by-type or introduce a second protocol) vs. **(iii) rule slices out of
Iterator** under the current protocol entirely. This is a brief, not a
spec. Round S6d-4 closed (ii) as well: both protocol shapes funnel into
the same missing IR-layout + checker capability. **Recommendation (per the
maintainer's long-term-consistency preference, 260907): do not fork the
protocol and do not hack a carve-out — defer S6d behind a new,
design-bearing prerequisite slice ("reference-bearing aggregates": slice-
shaped fields in aggregates with a storage-class/taint rule extending the
no-stored-reference discipline to the containing value, plus the layout
work). Once that capability exists, the existing Step-row protocol works
for slices unchanged — one protocol, no re-litigation of S8's R1, no
second trait.** The alternative (redesigning the protocol around 1-output
shapes to dodge return bundles) would contort every existing impl to
accommodate the one target class the type system cannot yet express, which
is the inconsistency the preference rules out.
