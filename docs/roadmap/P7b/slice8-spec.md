# Spec: P7b.S8 — Linear iterators (HKT as the associated-type substitute)

**Status:** Draft
**Created:** 2026-09-06
**Discovery:** [slice8-brief](./slice8-brief.md) (rulings R1–R4 + the S8b carve-out, decided 260905 — settled, do not reopen) and the verbatim probe log [slice8-probes](./slice8-probes.md) (round P8, four workers, P8-1..P8-6). Roadmap entry: [P7b-higher-kinded-types.md](../P7b-higher-kinded-types.md), section "P7b.S8". Base: `54414cb` (P7b.S7 landed and merged; suite green — fmt, clippy, 3200+ tests).

> **Anchor provenance.** The probe log's `path:line` citations are `ae6fdd7`-era. Every
> anchor in this spec was re-verified against the current tree (`54414cb`) on 2026-09-06.
> Where the probe era and the current tree disagree, this spec's numbers win. S7 also
> **reworded** the member-shape fence message (now "...plus a trait-var-headed
> application, in a plain slot or a quotation row..."); S8's diagnostic pins use the
> current text (`src/parser.rs:446`), not the probe log's. Second pass (review round 1,
> 260906): all anchors re-verified again and the round's corrections folded in; every
> `path:line` below is current as of this revision.

## Problem Statement

Sooth has container types (`List`, `Option` in `lib/core/`) and a working trait system
with shared bounds (S2/S4/S6/S7), but no iteration protocol: there is no way to write
one generic `for_each` or `fold` that drains a `List` or a count-up `Range` without
hand-rolling a per-type recursive word. Associated types stay out of scope by design —
the roadmap's answer is to make the iterator itself the type constructor
(`trait: Iterator['It: * -> *] : next ( 'It['T] -- Step['T 'It['T]] ) ;`), with
linearity *as* the protocol: `next` consumes the iterator and yields
element-plus-remainder, `drop` is the explicit destructor, and the exhausted case is
exactly the question of where the final drop lives. The probe round (P8, 260905)
refuted the old "delta ≈ zero" premise and located the real blockers: the trait cannot
even be **declared** today (the member-row gate rejects every ctor-headed App,
`src/parser.rs:403`), and a count-up `Range` has **no legal impl target** (the S2-6
concrete-impl-target fence, `src/ast.rs:2137` via `src/parser.rs:4334`). Until these
two compiler deltas land, the Iterator trait is undeclarable and the roadmap exit —
`next`/`for_each`/`fold` through a bound over List and Range goldens — is unreachable.

## Requirements

Rulings R1–R4 (260905) are settled user decisions; they are encoded below as
requirements, not reopened.

- **REQ-1.** (Delta A — member-row gate lift, prerequisite for the trait.) The
  member-row gate `member_shape_is_supported` (`src/parser.rs:381`) must admit a
  ctor-headed application (`PolyType::Generic`) in a trait member signature when every
  one of its type arguments is itself a supported member-row shape, recursing through
  the existing arms — **type arguments only**: any variable length argument inside the
  ctor application (`Len::Var`, e.g. a `Buf['T 'N]` member row) stays rejected with the
  same message, mirroring the Array arm's treatment (`build_member_var_union` maps ty
  vars only, `src/parser.rs:668-745`; the grounded `PolySig` takes its `len_var_names`
  from the target, `src/parser.rs:4415`, so a member's own `Len::Var` would dangle and
  panic the diagnostic renderer's `sig.len_var_names[*id]` indexing,
  `src/check/poly.rs:11946`/`:11993`). Consequently `Option['T]`, `Step['T 'It['T]]`,
  and the non-nested `Step['T 'T]` must all declare (probe P8-1a/P8-1c/b7 refusals
  today).
- **REQ-2.** The gate lift must stay admission-shaped. Still rejected, byte-exact: a
  plain-slot member-local-headed application (`'G['T]`, non-nested) and a ctor-headed
  row **containing** a member-local-headed App argument (`Step['G['T] 'F['T]]` — the
  lifted arm's recursion into args keeps this fenced post-lift) raise
  `unsupported_trait_member_shape_error` (S7-reworded text, `src/parser.rs:446`); a
  row-nested member-local-headed App (`'G['U]`) raises `app_in_member_quotation_row_error`
  (`src/parser.rs:459`; S7's G4b precedent, `tests/phase7b_slice7.rs:180`); plus
  `PolyType::OwnedCell`, `QuotLit`, `GenericVariant`, var-length arrays, and a ctor
  application with a variable length argument (REQ-1). A nested `'F['G['T]]` member row
  is **legal today** (`ground_member_poly` supports member-local-headed nested
  applications, `src/ast.rs:2338-2343`) and stays legal — it is not on the reject list.
- **REQ-3.** (R1 — the Step row is the protocol.) Core must gain the protocol vehicle:
  `type: Step['T 'Rest] | Done | More 'T 'Rest ;` and the trait
  `trait: Iterator['It: * -> *] : next ( 'It['T] -- Step['T 'It['T]] ) ;` in a new
  `no_std` core-layer module. `Done` carries nothing. The Option row
  (`next ( 'It['T] -- Option['T] 'It['T] )`) is the recorded rejected alternative and
  must not be implemented (arm parity forces the dead iterator to the caller and the
  final drop to every `None` path — probe P8-1d1b/d2).
- **REQ-4.** (List impl, no wall.) The system must provide
  `impl: Iterator for List` (generic target, today's generic path): the `Cons` arm
  destructures and packs `More` (element deepest, remainder on top); the exhausted
  `Nil` arm consumes the matched List and returns only the `Done` shell — the final
  drop lives inside `next`'s `Done` arm as the one canonical site, and no remainder is
  constructed (so the S6 construction wall is never reached).
- **REQ-5.** (R4 — consumers through the bound, List.) The system must provide generic
  `for_each['It: Iterator 'T] ( 'It['T] [ 'T -- ] -- )` and a generic `fold` (accumulator
  under the element; `[ 'A 'T -- 'A ]`), written once against the Iterator bound,
  self-recursive with the self-call in tail position (the P7.S3g loop-back-edge
  precedent, `src/ir/driver.rs:1093`), dispatching `next` through the bound over the
  List impl.
- **REQ-6.** (Bound-dispatch verification — the explicit early verification point.)
  A poly body's member call through an Iterator bound over a ctor-headed compound
  member return (the Step row) must be **measured** the moment REQ-1 lands: if
  `poly_cross_call_unsupported_error` (`src/check/poly.rs:4371`) fences it, the phase
  stops and escalates with a verbatim repro — no silent reshape of `next`, no
  unsanctioned lift of the cross-call gate. The S4 `q map` precedent
  (`shared_bound_poly_word_dispatches_over_the_real_core_option`,
  `tests/phase7b_slice4.rs:199`, `q map` at `:212-213`) predicts it works; it is
  unmeasured for the Step row until REQ-1 lands.
- **REQ-7.** (Delta B — the S2-6 concrete-impl-target lift, R2.) An impl target that
  is a fully-applied ctor application with all-concrete arguments
  (`impl: Iterator for Range[i64]`) must become legal: `'It` unifies with the ctor
  head, the member's App arguments bind to the target's concrete arguments (the union
  builder already renders concrete slots as themselves, `src/parser.rs:668`), and the
  member body must check **monomorphically** — a mono member word with a concrete
  `StackEffect`, not a `PolySig`. This is the pinned representation decision (rationale
  and the considered-and-rejected alternative recorded under Solution Approach → Delta
  B): the existing concrete path already produces concrete member words (`impl: Ord for
  i64`, `lib/core/cmp.sth`, synthesized at `src/parser.rs:4346-4360`), and a
  fully-applied ctor application with all-concrete args is semantically as concrete as
  `i64`. The D5 borrow gate (`src/check/poly.rs:6579-6588`, which stays untouched)
  independently forbids checking the member as a *generic* poly body — a `Generic`-typed
  local is not a borrowable aggregate — but does not by itself select the mono-word
  representation. The documented fallback, only if phase 3's obligation/mint measurement
  cannot land within S8: route lifted targets through the existing generic path, keeping
  the member word `poly: Some` with a var-free fully-concrete grounded `PolySig`
  (threads today's dispatch/mint machinery unchanged; satisfies D5).
- **REQ-8.** (Fence boundaries byte-exact.) Mixed and partial applications keep
  today's behavior: an App-headed member against a plain concrete target (`for i64`)
  still raises `member_app_concrete_target_error` (`src/ast.rs:2095` text byte-exact
  via `fence_member_app_against_concrete_target`, `src/ast.rs:2137`, called from
  `src/parser.rs:4334`); an App-headed impl target still raises
  `impl_target_app_unsupported_error` (`src/parser.rs:902`); an applied-var ctor
  target (`List['L]`) keeps the generic path with a poly member word.
- **REQ-9.** (R2 — the Range golden.) Core must gain
  `type: Range['T] cur 'T limit 'T ;` (stays generic; no phantom parameters — those
  are dead by design, `phantom_ty_var_error`, `src/parser.rs:2519`) and
  `impl: Iterator for Range[i64]`: a count-up `next` doing arithmetic on `i64`
  (`1 add`), constructing the advanced `Range` inside the `More` arm (plain fields —
  the wall is not reached, probe P8-2a) and dropping the consumed iterator inside the
  `Done` arm. `next` over `Range[i64]` must dispatch at a plain mono call site. No D5
  changes, no numeric trait, no phantom params.
- **REQ-10.** (R4 — consumers through the bound, Range.) `for_each` and `fold` must
  drain and fold `Range[i64]` through the Iterator bound (0 1 2; sum 3), with no
  per-impl copies of the consumers.
- **REQ-11.** (IR evidence — recorded, no verdict.) The suite must contain an
  automated pin that a consuming loop lowers to **one frame whose self-call is a
  back-edge** (pattern: `poly_self_tail_call_lowers_to_loop_back_edge`,
  `src/ir/driver.rs:1093`), and the written record must state the P8-6 facts as
  evidence only — the loop is one frame, `next`/`list-next` is a real monomorphized
  frame (not spliced), a two-consumer chain is two dedicated frames — with no fusion
  verdict ("one frame total" is true of the loop, not of loop+`next`).
- **REQ-12.** (Write-downs + roadmap correction.) The exhausted-case ruling (R1: Step
  row, `Done` carries nothing, the final drop inside `next`'s `Done` arm; the Option
  row rejected) and the fusion evidence must be written into the roadmap's S8 entry,
  and the entry corrected at the final phase per house convention (exit wording to the
  landed `for_each`/`fold` outcome, implemented-reference link to this spec).
- **REQ-13.** (R3 / S8b carve-out — the wall stays.) The S6 construction-wall arm —
  the `poly_bind_construction_arg` catch-all at `src/check/poly.rs:6116-6117` (fn at
  `:5998`) — must remain untouched, and the wall witness
  `monoid_for_list_append_construction_wall_is_recorded` (`tests/phase7b_slice6.rs:410`)
  must stay green. The wall fix plus per-impl traitful `List` members (`map`,
  `append`) are **P7b.S8b**, out of this slice.

Non-functional requirements:

- **REQ-NFR1.** (Green gate.) Every phase exits with `cargo fmt --check &&
  cargo clippy -- -D warnings && cargo test` green (3200+ baseline tests plus the new
  goldens); the final phase re-verifies the whole gate. Diagnostics are behaviour: any
  new error text is measured-then-pinned byte-exact.
- **REQ-NFR2.** (Stage discipline.) Both compiler deltas are parser/check-stage only:
  no IR or lowering changes (`src/ir/` diff-empty), the QBE backend untouched, `Ptr[T]`
  stays an opaque handle. The target-fold interception lives in the **impl-target
  path only** — `raw_to_poly_type`'s Generic-arm fold (`src/parser.rs:5655-5729`) is
  not changed globally, because ordinary signatures depend on the Concrete fold for
  instantiation mint-sharing (S4-1).
- **REQ-NFR3.** (Layering.) The new protocol module is `core`-layer `no_std` (no
  `hosted` imports), registered in `lib/core/sooth.pkg`'s module list, following the
  `lib/core/cmp.sth` house pattern for a core trait module. Nothing becomes implicit:
  consumers `import:` it explicitly.
- **REQ-NFR4.** (Linear spine respected.) `next` consumes the iterator exactly once;
  `Done` carries nothing; no auto-drop is added (never-moved frame locals keep today's
  implicit reclamation, probe P8-1d4/n1); the enforced teeth stay compile-time —
  leaving a `Step` value undropped across dispatch arms is a compile error,
  measured-then-pinned through the new protocol: the variant-escape rule fires
  first (`poly_eliminator_variant_escape_error`, `src/check/poly.rs:11544`; mono
  twin `eliminator_variant_escape_error`, `src/check.rs:2832`), and if the arms
  instead differ only in stack shape, the arm-parity join error
  (`src/check.rs:2983`) applies.

## Success Criteria

- [ ] `trait: Iterator['It: * -> *] : next ( 'It['T] -- Step['T 'It['T]] ) ;` compiles
      (today it fails with the unsupported-signature message — probe P8-1a/P8-1c).
- [ ] A List built as 1,2,3 drains via `for_each` through the Iterator bound, printing
      `1\n2\n3`, exit 0; `0 [ add ] fold` over it yields 6 (house spelling — accumulator
      under element; `lib/core/combinators.sth:53`, `examples/array_totals.sth:25`).
- [ ] `0 3 Range` drains via `for_each` printing `0\n1\n2`; `fold` sums it to 3.
- [ ] `next` dispatches over `Range[i64]` at a plain mono call site (one `More`, then
      `Done`, observable).
- [ ] `impl: Iterator for i64` with the App-headed member still fails with today's
      byte-exact S2-6 message; an App-headed impl target still fails with
      `impl_target_app_unsupported_error`; `List['L]` as a target still yields a poly
      member word.
- [ ] A plain-slot member-local-headed App (`'G['T]`), a ctor-headed row containing one
      (`Step['G['T] 'F['T]]`), and a row-nested one (`'G['U]`) still fail with the
      byte-exact S7-reworded messages (`src/parser.rs:446` / `:459`); a nested
      `'F['G['T]]` member row still builds.
- [ ] An IR-level check in the suite shows the consuming loop's monomorphized
      `for_each` is one frame with a self back-edge; the written record states the
      loop+`next` facts with no fusion verdict.
- [ ] `docs/roadmap/P7b-higher-kinded-types.md`'s S8 entry carries the exhausted-case
      ruling, the fusion evidence, the `for_each`/`fold` exit wording, and a link to
      this spec; a growth-structure re-check is documented.
- [ ] `src/check/poly.rs`'s construction-wall catch-all is diff-empty and the S6 wall
      witness test passes.
- [ ] Full gate green: fmt, clippy `-D warnings`, all tests (3200+ baseline + new).

## Scope & Boundaries

**In scope:**

- Delta A: the `member_shape_is_supported` `Generic` arm lift (ctor-headed Apps in
  member rows, recursively argued).
- The core protocol module: `Step['T 'Rest] | Done | More 'T 'Rest`, the
  `Iterator['It: * -> *]` trait with the Step-row `next`, impls for `List` (generic
  target) and `Range[i64]` (R2), and the generic `for_each`/`fold` consumers.
- Delta B: the S2-6 concrete-impl-target lift — fully-applied (all-concrete-arg) ctor
  targets legal; App-headed members grounded as monomorphic instantiations.
- Goldens (`tests/phase7b_slice8.rs`, new), diagnostic pins (measure-then-pin,
  byte-exact), the one-frame IR pin, the write-downs, the roadmap correction, and the
  growth-structure re-check.

**Out of scope** (per the brief's "Explicitly out of scope" and the decided rulings):

- **P7b.S8b (carved out, decided 260905):** the S6 construction-wall fix
  (`poly_bind_construction_arg`'s bare-`Generic` `^Self['T]` self-reference-field arm,
  `src/check/poly.rs:6116-6117`) plus per-impl traitful `List` members (`map`,
  `append`). Nothing in S8's Step-row `next` shapes needs the wall (probe P8-2a/2b/2c).
- Associated types / GATs — the slice exists to show they are not needed.
- Borrow-based iteration (`&!`, lifetimes, exclusivity) — linearity replaces it.
- Lazy/streaming iterators and an adaptor library (`zip`/`take`/`rev`/...) — no free
  library work; the trait is the dogfood, not a stdlib slice. `map` is **not** in S8
  (R4): a bound-generic body cannot produce `'It['U]` (P8-5b) and cannot `dup` the
  abstract iterator (`poly_copy_generic_error`, `src/check/poly.rs:10954`).
- Fusion as an answered question — evidence recorded (REQ-11), ruling deferred.
- Iterator for `array` — S6b territory.
- Any change to `Option`'s shape or `core::option`'s surface — consumed as-is.
- The D5 borrow gate (`src/check/poly.rs:6579`), any arithmetic trait, and phantom
  parameters (R2's scope fences — the generic-target route is not taken).

## Solution Approach

The slice is two compiler deltas plus one core module, in that dependency order.

**Delta A (member-row gate).** `member_shape_is_supported` (`src/parser.rs:381`) is a
pure shape predicate enforced from `parse_trait_member_effect`'s post-parse sig walk
(`src/parser.rs:3959-3986`). Today its combined rejection arm bundles
`PolyType::Generic { .. }` with `OwnedCell`/`QuotLit`/`GenericVariant`
(`src/parser.rs:402-407`); S8 splits `Generic` out and admits it when every type
argument is itself supported — the same recursion the `Quotation` arm gained in S7,
**type arguments only** (a `Len::Var` inside the ctor application stays rejected,
REQ-1). This admits all three refuted rows at once (`Option['T]`, `Step['T 'It['T]]`,
`Step['T 'T]`) and changes nothing about grounding; a nested `'F['G['T]]` member row is
legal today and stays so (`ground_member_poly`'s `head != 0` arm supports
member-local-headed nested applications, `src/ast.rs:2338-2343`). The lift is
admission-only, so its safety must be swept: every newly-admitted shape either grounds
through the existing generic path or fails with a located error, never a panic (pin one
non-Iterator admitted shape, e.g. the probe's Interlude-B `Box2['T]` member row; a
`Buf['T 'N]` var-length ctor arg; and the panics below). Two panics **are** reachable
post-lift, both from `ground_member_type`'s missing `Generic` case (`_ =>
unreachable!`, `src/ast.rs:2209-2211`): (1) a ctor-headed member row against a
**concrete** impl target passes the S2-6 fence (`member_ty_mentions_app`,
`src/ast.rs:2113`, is false for an App-free `Generic`) and reaches it via
`parse_impl_member_body`'s concrete branch — fenced parser-side (`src/parser.rs:4325`),
a located, measured-then-pinned error; (2) `unsatisfied_user_bound_error`
(`src/check/poly.rs:9047`) renders member signatures through
`try_ground_member_type`, whose fallthrough (`src/ast.rs:2245`) hands a `Generic` to
the same `unreachable!` — live the moment a bound over a trait with a ctor-headed
member row is instantiated at a type with no impl (found in phase-1 review; it fires
on the shipped `core::iterator` itself, and it is exactly phase 2's
bound-instantiated-at-unsupported-type case). Fence (2) is a defensive
`PolyType::Generic { .. } => None` arm in `try_ground_member_type` (mirroring the App
arm's fallback at `src/ast.rs:2239`) — the one deliberate `src/ast.rs` change phase 1
makes, so the caller's existing missing-impl diagnostic fires instead of a panic.

**The protocol (R1).** One new core module carries the whole protocol: the two-param
`Step` enum, the single-type-variable `Iterator` trait (multi-var headers are fenced
by design — `multi_variable_trait_error`, `src/parser.rs:493`), the List impl, the
Range type and impl, and the two consumers. List's `next` works entirely on today's
generic-target path: enum destructuring over a poly target is permitted, and
enum-ctor construction (`More`/`Done`) inside a member body is the verified S6
precedent (probe P8-2c) — and because `Done` carries nothing, the exhausted `Nil` arm
needs no remainder construction at all, which is exactly why S8's shapes never touch
the S6 wall (R3). The List impl lives in the protocol module (the module owns the
trait; S9/S10's cross-module impl machinery exists for concrete targets, so
implementing a foreign type with an own-module trait is expected to pass
`check_impl_decls` — verified as a phase-1 exit item).

**Delta B (the S2-6 lift, R2).** The decisive constraint is R2's own scope fence: the
D5 borrow gate stays untouched, and D5 only admits `Concrete` aggregates as borrowable
locals (`src/check/poly.rs:6579-6588`). A `Range[i64]` local in *poly* space is
`PolyType::Generic` — not borrowable — so a poly-bodied member can never read `cur`.
Therefore the lifted target **must** ground its members monomorphically — D5 forbids
the *generic* poly body; the mono-word-vs-degenerate-poly representation choice is made
on representation-matches-semantics grounds (REQ-7). The mechanism:
intercept the fold in the impl-target path (`parse_impl_target`,
`src/parser.rs:4085-4095`) so a fully-applied all-concrete ctor application keeps its
`PolyType::Generic { args: all-concrete }` pattern instead of collapsing to
`Concrete` — which preserves ctor identity for diagnostics and dispatch, and leaves
`raw_to_poly_type`'s global fold (and S4-1's mint-sharing) untouched (REQ-NFR2). Then
`parse_impl_member_body` (`src/parser.rs:4266`) routes such targets through the
existing grounding machinery — `build_member_var_union` (`src/parser.rs:668`) already
binds a member local named in a dispatchable input's application argument to the
target slot's contents (a concrete slot renders as itself), and `ground_member_poly`'s
App arm (`src/ast.rs:2337-2400`) already dissolves `'It['T]` against a `Generic`
target — and, seeing the grounded signature is var-free, synthesizes the concrete
path's mono member word (`poly: None`, concrete `StackEffect`, `src/parser.rs:4346-4360`)
via the same instantiation machinery the fold uses. The S2-6 fence
(`fence_member_app_against_concrete_target`, `src/ast.rs:2137`) stays exactly where it
is for everything else: plain concrete targets (`for i64`), App-headed targets, and
mixed/partial applications keep today's byte-exact behavior (REQ-8, S6b-style
fencing).

**Delta B dispatch — the answered question and the real work.** The matcher side needs
nothing: `match_impl_target_rec` (`src/check/poly.rs:9131`) already recursively
compares a concrete-arg `Generic` pattern against a mono operand instantiation,
argument-by-argument and length-by-length (the `Generic` arm `:9235-9319` zips args and
len args; `CtorImage` operands are skipped by design, S2-8), exercised by
`match_impl_target_generic_zips_len_args_against_concrete_length`
(`src/check/poly.rs:20051`). The real gap is downstream of the match, where both
continuations key on the target's concreteness. (a) Mono call sites:
`resolve_mono_member_call` (`src/check/poly.rs:2233`) splits on
`imp.target.is_concrete()` (`:2406`); a lifted target keeps a `Generic` pattern and
falls into the else branch, whose `debug_assert!` requires the member word in
`poly_env` (`:2545-2549` — `poly_env` is built only from `poly: Some` words,
`src/check.rs:703-707`), so REQ-7's `poly: None` mono word panics it in debug builds.
The concrete branch is also unusable: it re-grounds the *trait's* member signature via
`ground_member_type`, whose App arm and fallthrough are both `unreachable!`
(`src/ast.rs:2206-2211`). Required: a lifted-target arm that checks call-site slots
against the mono grounded effect and records the dispatch symbol span-keyed,
`builtin_overloads`-style — the concrete branch's own recording scheme
(`src/check/poly.rs:2434-2437`) — check-stage only, no IR change. (b) Bound dispatch
(phase 4's Range consumers): `resolve_user_bound` sets
`is_generic = !imp.target.is_concrete()` (`:8738`); the generic-winner arm pushes
`(word, subst)` into `impl_monos` (`:8938-8943`); `impl_mono_seed` (`:7927`) requires
`word.poly = Some` (`:7939`) — a mono member word yields `Ok(None)` and the dispatch
symbol never gets a lowered body. Required: the obligation/mint path must seed mono
member words for lifted targets, or lifted targets route through a concrete-winner-style
path — exact mechanism measured first in phase 3, fenced to fully-applied all-concrete
targets. The considered-and-rejected alternative (and the documented fallback if that
obligation/mint work cannot land within S8): route lifted targets through the existing
generic path, keeping the member word `poly: Some` with a var-free fully-concrete
grounded `PolySig` — it threads today's dispatch/mint machinery unchanged
(`impl_mono_seed` `:7939`; `resolve_user_bound`'s generic-winner arm `:8938-8943`) and
satisfies D5 (a var-free concrete sig instantiates locals to `Concrete` aggregates) —
but it is a degenerate poly word: representation must match semantics, and the mono
word plus the new arm is permanent reusable infrastructure (S8b's traitful List
members, any future ctor-spelled concrete target) with diagnostics grounded in real
concrete types.

**Consumers (R4).** `for_each` and `fold` are ordinary poly words with one
`Iterator` bound, self-recursive with the self-call in tail position so the P7.S3g
self-tail-call transform makes the loop a single frame with a back-edge
(`src/ir/driver.rs:1093`). Member dispatch through a bound has the S4 `q map`
precedent (`tests/phase7b_slice4.rs:212-213`) — `map` returns the compound `'F['U]`
and dispatches fine — but the Step row's ctor-headed compound return is unmeasured,
which is why REQ-6 makes it the first thing measured after Delta A, with
`poly_cross_call_unsupported_error` (`src/check/poly.rs:4371`) as the named risk.

## Codebase Map

All anchors re-verified on base `54414cb`, 2026-09-06.

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/parser.rs:381` | `member_shape_is_supported()` | Delta A target; `PolyType::Generic` arm at `:403` (inside the combined false arm `:402-407`) lifts |
| `src/parser.rs:446` | `unsupported_trait_member_shape_error()` | S7-reworded message — REQ-2/REQ-NFR1 pin against this text |
| `src/parser.rs:459` | `app_in_member_quotation_row_error()` | S7's row fence — stays byte-unchanged |
| `src/parser.rs:3927` | `parse_trait_member_effect()` | Enforcement loop `:3959-3986` picks up the lift automatically |
| `src/parser.rs:4085` | `parse_impl_target()` | Delta B interception at the fold `:4094`; App-head target fence `:4095-4099` stays |
| `src/parser.rs:902` | `impl_target_app_unsupported_error()` | App-headed-target message — REQ-8 pin |
| `src/parser.rs:4126` | `parse_impl_target_pattern()` | Ctor intercept with `UnderApplication::PadImplTarget` (`:4152`) — the target ctor path the lift rides |
| `src/parser.rs:4266` | `parse_impl_member_body()` | Concrete branch `:4325`; S2-6 fence call `:4334`; the concrete path's mono member-word synthesis `:4346-4360` (what Delta B reuses); generic/poly path `:4364-4420` — Delta B adds the fully-applied-ctor mono grounding here |
| `src/parser.rs:668` | `build_member_var_union()` | Binds member locals to target slots; concrete slots render as themselves (verified) |
| `src/parser.rs:4415` | `len_var_names: target.len_var_names.clone()` | The grounded `PolySig` takes len vars from the target — a member's own `Len::Var` would dangle (REQ-1) |
| `src/parser.rs:5565` | `raw_to_poly_type()` | Generic-arm fold `:5655-5729` (all-concrete collapse `:5681-5715`) — **do not change globally** (REQ-NFR2); only the impl-target path intercepts |
| `src/parser.rs:2519` | `phantom_ty_var_error()` | Phantom parameters dead by design — Range keeps real fields |
| `src/parser.rs:493` | `multi_variable_trait_error()` | Single-type-variable traits only — the trait header shape is fixed |
| `src/ast.rs:2095` | `member_app_concrete_target_error()` | S2-6 message — REQ-8 byte-exact pin for non-lifted targets |
| `src/ast.rs:2137` | `fence_member_app_against_concrete_target()` | The S2-6 fence (scan `member_ty_mentions_app`, `:2113`), called from `src/parser.rs:4334` |
| `src/ast.rs:2072` | `member_app_abstract_target_error()` | The abstract-target twin (unaffected) |
| `src/ast.rs:2051` | `member_app_arity_error()` | Arity gate on the App dissolve — reused by the lift |
| `src/ast.rs:2283` | `ground_member_poly()` | App arm `:2337-2400` dissolves `'It['T]` against a `Generic` target — the dissolve Delta B reuses; `head != 0` nested-App support `:2338-2343` is why `'F['G['T]]` rows are legal |
| `src/ast.rs:2168` | `ground_member_type()` | Concrete-branch re-grounding; App arm + fallthrough both `unreachable!` (`:2206-2211`) — post-lift panic site (1); fenced parser-side in phase 1 |
| `src/ast.rs:2233` | `try_ground_member_type()` | Member-sig rendering helper (`unsatisfied_user_bound_error` path, `poly.rs:9047`); fallthrough `:2245` was panic site (2); phase 1 adds the `Generic => None` fallback (mirrors the App arm `:2239`) |
| `src/ast.rs:2483` | `ImplTarget::is_concrete()` | `matches!(pattern, PolyType::Concrete(_))` — the test the lifted targets stop satisfying (no change needed) |
| `src/check/poly.rs:5998` | `poly_bind_construction_arg()` | **Do not touch** (S8b): catch-all `:6116-6117` is the S6 wall |
| `src/check/poly.rs:6235` | `poly_construct_generic()` | Poly construction path — plain-field ctors construct fine in member bodies (P8-2a) |
| `src/check/poly.rs:4371` | `poly_cross_call_unsupported_error()` | Named risk for REQ-6 (compound-return cross-call fence) |
| `src/check/poly.rs:6579` | D5 `is_aggregate` match | Why the Range member must be mono; message `poly_borrow_of_non_aggregate_local_error` `:11250` — **do not extend** |
| `src/check/poly.rs:8520` | `find_bound_impl()` | Member-call dispatch through the bound |
| `src/check/poly.rs:2233` | `resolve_mono_member_call()` | Mono member-call dispatch; `is_concrete()` split `:2406`; else-branch `debug_assert!` `:2545-2549`; concrete branch records `builtin_overloads` `:2434-2437` — the lifted-target arm lands here (phase 3) |
| `src/check/poly.rs:7927` | `impl_mono_seed()` | Requires `word.poly = Some` (`:7939`) — mono-word seeding for lifted targets lands here or beside it (phase 3/4) |
| `src/check/poly.rs:8738` / `:8938-8943` | `resolve_user_bound()` | `is_generic = !is_concrete()` test + generic-winner arm pushing into `impl_monos` — bound-dispatch continuation (phase 4) |
| `src/check/poly.rs:9131` | `match_impl_target_rec()` | Impl-target matching — the `Generic` arm `:9235-9319` **already** recursively compares concrete-arg patterns (args + len args; unit test `:20051`); no extension needed (OQ-1 answered) |
| `src/check/poly.rs:11946` / `:11993` | diagnostic rendering | Indexes `sig.len_var_names[*id]` — OOB panic if a member's own `Len::Var` dangles (REQ-1) |
| `src/check/poly.rs:10954` | `poly_copy_generic_error()` | Conservative `dup` fence over bounds — consumers never duplicate the iterator |
| `src/check/poly.rs:11551` | variant-escape rule (poly arm) | Matched variants cannot escape their arm — the Nil arm's discipline |
| `src/check.rs:703` | `poly_env` construction | Built only from `poly: Some` words — why a mono member word panics `resolve_mono_member_call`'s else-branch assert |
| `src/check/declarations.rs:588` | orphan rule (`impl_target_module` `:500`) | An impl is legal iff its module is the trait's declaring module or the target ctor's module — `impl: Iterator for List` in `iterator.sth` satisfies the trait-module arm; fixture-declared impls of the core trait would not (OQ-2 fallback) |
| `src/check.rs:2983` | arm-parity join error | The linearity teeth (d5 evidence) — REQ-NFR4 pin site |
| `src/ir/driver.rs:1093` | `poly_self_tail_call_lowers_to_loop_back_edge` | Precedent + pattern for REQ-11's one-frame pin |
| `src/driver.rs:897` | `emit_ssa_with_manifest()` | IR evidence capture path (P8-6 method) |
| `lib/core/list.sth:1` | `List['T]` (Nil/Cons, `rest ^List['T]`) | List impl target — destructure-only |
| `lib/core/option.sth:1` | `Option['T]` | Consumed as-is; the Option-row rejected alternative is a record only |
| `lib/core/cmp.sth` | `Ord` trait + derived members | House pattern for a core trait module (header comment, exports) |
| `lib/core/sooth.pkg` | `module:` list | Gains the new protocol module (append `iterator`) |
| `lib/core/iterator.sth` (new) | — | The protocol module: Step, Iterator, List impl, Range, Range[i64] impl, for_each, fold |
| `tests/fixtures/sooth.pkg` | fixture manifest | Invocation contract for CLI-shaped fixtures (`--manifest`) |
| `tests/phase7b_slice8.rs` (new) | — | S8 goldens + diagnostic pins; `build_run_keep` is a **per-file helper** (`tests/phase7b_slice4.rs:70` is the pattern; also `:71` in slice3, `:91` in slice7); `tests/common/mod.rs` provides `fixture_manifest`/`manifest_for`/`fixture_package` |
| `tests/phase7b_slice4.rs:199` | `shared_bound_poly_word_dispatches_over_the_real_core_option` | The bound-dispatch (`q map`, `:212-213`) precedent |
| `tests/phase7b_slice6.rs:410` | `monoid_for_list_append_construction_wall_is_recorded` | Wall witness — must stay green (REQ-13) |

Load-bearing constraints:

- `src/parser.rs:5655-5729` (`raw_to_poly_type`'s Generic-arm fold): do not change
  globally — ordinary signatures fold `Range[i64]` to `Concrete` and share the S4-1
  mint; the interception is impl-target-scoped only.
- `src/check/poly.rs:6116-6117`: the S6 wall — S8b's, not S8's (REQ-13).
- `src/check/poly.rs:6579-6588`: the D5 aggregate gate — not extended; it is the
  *reason* Delta B grounds monomorphically.
- Single-type-variable traits (`src/parser.rs:493`): the `'O['T]` two-var alternative
  is dead by design (probe P8-4a2) — do not revisit.

## Open Questions

- [x] ~~Exhausted case: Option row vs Step row~~ — **R1 (260905):** Step row; `Done`
  carries nothing; the final drop inside `next`'s `Done` arm. Option row recorded as
  the rejected alternative.
- [x] ~~Range impl target: phantom vs concrete-target lift vs demote~~ — **R2
  (260905):** the S2-6 lift lands in S8; `impl: Iterator for Range[i64]` is a golden.
- [x] ~~Does S8 carry the S6 construction-wall fix?~~ — **R3 (260905):** no; wall fix
  - traitful List `map`/`append` carved out to P7b.S8b.
- [x] ~~Does the exit mean `map`?~~ — **R4 (260905):** no; consuming `for_each` +
  `fold` through the bound.
- [x] ~~Does `match_impl_target_rec` already compare a concrete-arg Generic pattern
  against a mono operand instantiation, or must the comparison be added?~~ —
  **Answered by the code (review round 1, 260906):** yes — the `Generic` arm
  (`src/check/poly.rs:9235-9319`) recursively compares args and len args against the
  operand instantiation (unit test
  `match_impl_target_generic_zips_len_args_against_concrete_length`, `:20051`);
  `CtorImage` operands are skipped by design (S2-8). No matcher extension is needed or
  permitted. Phase 3's check-stage work is the continuations instead (REQ-7, Solution
  Approach → Delta B dispatch).
- [ ] Does `check_impl_decls`' placement rule accept `impl: Iterator for List` in
  `lib/core/iterator.sth` (own-module trait, foreign type)? Expected yes — verified by
  reading the rule (`src/check/declarations.rs:588`: legal iff the impl's module is the
  trait's declaring module or the target ctor's module, `impl_target_module` `:500`;
  `cmp.sth`'s `impl: Ord for i64` satisfies the trait-module arm). Phase 1 verifies.
  The originally recorded fallback (fixture-declared impls of the core trait) is **not
  viable** — it violates the orphan rule. Viable escapes, preferred first: the fixture
  declares its **own trait+impl pair** (the S4-5 precedent, `tests/phase7b_slice4.rs` —
  zero lib churn); or the impls move into `list.sth`; or the fixture declares a twin
  target type.
- [x] ~~The exact `import:` surface a consumer module needs for the member name
  (`next`) to resolve through the bound~~ — **answered in phase-1 review (260906,
  measured):** `import: core::iterator | Step Done More Iterator | ;` — import the
  trait (type + ctors + trait name), do NOT name the member; bare `next` then
  resolves at the dispatch site (verified end-to-end: selective-import drain of
  `List[i64]` builds and runs). Naming `next` in the import list errors; omitting
  the import errors `unknown word next`.

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Member dispatch through the bound is fenced by `poly_cross_call_unsupported_error` (Step-row compound return is unmeasured) | Low–Med | REQ-6 makes it the first measurement after Delta A; the `q map` precedent (S4-5) predicts success; if fenced: stop, escalate with verbatim repro + evidence (named site `src/check/poly.rs:4371`) — no silent reshape of `next`, no unsanctioned gate lift |
| The `is_concrete()`-keyed dispatch continuations reject the mono member word (`resolve_mono_member_call`'s else-branch `debug_assert!` `:2545-2549`; `impl_mono_seed`'s `poly: Some` requirement `:7939`) → `Range[i64]` impl undebuggable-at-dispatch | Med | Phase 3 leads with the direct mono call-site golden; the lifted-target arm + mono-word seeding are the scoped fix, fenced to fully-applied targets (REQ-8 pins keep everything else byte-exact); the recorded option-B fallback (generic-path routing, `poly: Some` var-free `PolySig`) is the escape hatch |
| Delta B's fold interception leaks beyond impl targets and breaks ordinary-signature mint-sharing suite-wide | Low (high impact) | Interception lives only in `parse_impl_target` (`src/parser.rs:4085`); REQ-NFR2 + a regression pin (`Range[i64]` in an ordinary signature still folds/shares the mint) |
| The gate lift admits shapes whose grounding was never exercised (e.g. `Box2['T]` member rows now declare) and something downstream panics | Med | Lift is admission-only; phase 1 runs an admission-safety sweep — every newly-admitted shape grounds or fails located, never panics; pin one non-Iterator shape (`Box2['T]`), a var-length ctor arg (`Buf['T 'N]`, REQ-1), and the ctor-row-over-concrete-target shape (the `ground_member_type` `unreachable!`, fenced parser-side per REQ-1's panic-fence note) and the bound-over-unsupported-trait shape (the `try_ground_member_type` path, fenced by the `Generic => None` fallback) |
| Diagnostic drift: pins written against the probe log's superseded `ae6fdd7`-era message text | Low | This spec pins against the current S7-reworded text (`src/parser.rs:446`); measure-then-pin from the live binary (REQ-NFR1) |
| Sequential-phase merge friction in `member_shape_is_supported` (S7 + S8 edits accumulate in one function) | Low | Phases are strictly sequential (no parallel work declared); growth-structure re-check at phase 4 exit per CLAUDE.md |
| IR evidence over-read as a fusion claim | Low | REQ-11 records facts only (loop = one frame; `next` = real frame; chain = two frames); the roadmap wording stays "evidence recorded, ruling deferred" |

## Delivery Plan

### Phase 1: Declare the Iterator protocol — member-row gate lift + core Step/Iterator/List

- **Goal**: A program builds that declares the Step-row `Iterator` trait and calls
  `next` on a List at a mono call site, printing element-plus-remainder observables —
  the trait that errors today now declares.
- **Requirements Covered**: REQ-1, REQ-2, REQ-3, REQ-4, REQ-NFR3
- **Scope**:
  - Modify `src/parser.rs:403` (`member_shape_is_supported`, the `PolyType::Generic
    { .. }` arm inside the combined false arm `:402-407`): split `Generic` out and
    admit it when every type argument recurses to a supported shape, rejecting any
    `Len::Var` among the ctor application's arguments (REQ-1). Nothing else in
    the function changes; the enforcement loop (`:3959-3986`) picks it up.
  - Create `lib/core/iterator.sth` (register in `lib/core/sooth.pkg`'s `module:` list;
    pattern: `lib/core/cmp.sth`): the `Step['T 'Rest] | Done | More 'T 'Rest` enum
    (REQ-3), `trait: Iterator['It: * -> *] : next ( 'It['T] -- Step['T 'It['T]] ) ;`,
    and `impl: Iterator for List` (REQ-4: Cons arm destructures + packs `More`; Nil
    arm consumes and returns only `Done` — no remainder construction). Export the
    type, its ctors, and the trait; member names resolve through trait dispatch, not
    module exports (`export: ... next` names nothing — verified; `cmp.sth`
    precedent). Explicit imports only (no prelude change).
  - Create `tests/phase7b_slice8.rs` (`build_run_keep` is the per-file helper pattern,
    `tests/phase7b_slice4.rs:70`; golden style per the same file): (a) trait-declares
    - List `next` at a mono call site runs (the P8-4a3 shape through the trait
    member); (b) byte-exact pins: a plain-slot member-local-headed App (`'G['T]`) and
    a ctor-headed row containing one (`Step['G['T] 'F['T]]`) raise
    `unsupported_trait_member_shape_error` with the S7 text (`src/parser.rs:446`), a
    row-nested one (`'G['U]`) raises `app_in_member_quotation_row_error`
    (`src/parser.rs:459`), and a nested `'F['G['T]]` member row still builds;
    (c) admission-safety sweep: one newly-admitted non-Iterator shape (a `Box2['T]`
    member row over a dispatchable `'F['T]` input, per probe Interlude B) grounds or
    fails located — never panics; a `Buf['T 'N]` var-length ctor arg is rejected
    located (REQ-1); a ctor-headed member row over a **concrete** impl target
    (e.g. an `Option['T]` row against `for i64`) is fenced located in
    `parse_impl_member_body`'s concrete branch (`src/parser.rs:4325`); and a bound
    over a trait with a ctor-headed member row, instantiated at a type with no
    impl, fails located via the `try_ground_member_type` `Generic` fallback
    (`src/ast.rs:2233` region) — not the `ground_member_type` `unreachable!`
    (`src/ast.rs:2209-2211`), which both fences keep unreachable;
    (d) the cross-module-impl placement check
    (`impl: Iterator for List` in the protocol module) passes.
  - Unit tests beside the changed parser function for the new gate arm (happy +
    rejected shapes; naming per CLAUDE.md `thing_condition_expected`).
  - Explicitly out of scope for this phase: Range, Delta B, `for_each`/`fold`, any
    `src/check/` or `src/ir/` change, and all `src/ast.rs` changes except the one
    defensive arm above (`try_ground_member_type`'s `Generic` fallback — the
    review-discovered second panic path).
- **Entry Conditions**: Worktree green on `54414cb`; slice8-brief and slice8-probes
  read; R1–R4 treated as settled.
- **Exit Criteria / Verifiable Artifacts**:
  - `cargo test` green including the new `tests/phase7b_slice8.rs` goldens (a)–(d).
  - A CLI-fixture build of the declaring trait + List `next` exits 0 (the probe-log
    P8-1a fixture, fixed by the lift).
  - Existing S7 diagnostics byte-identical (pins b passes; full suite green = no
    churn elsewhere).
- **Parallelism**: SEQUENTIAL — first phase; nothing to run beside.
- **Relative Effort**: M — one gate arm plus a new core module, impl, and golden
  suite; the module and pins are the bulk, the gate change itself is small.
- **Difficulty**: `standard` — a scoped predicate change with existing enforcement;
  no concurrency, migration, or shared-control-flow work.
- **Open Questions / Blockers**: The placement-rule question (the first remaining
  open question) resolves here; if it fails, record and fall back to the fixture declaring its **own
  trait+impl pair** (the S4-5 pattern) for the phase-1 goldens — fixture-declared
  impls of the core trait violate the orphan rule (`src/check/declarations.rs:588`),
  so that older fallback is off the table (protocol module keeps Step + trait).

### Phase 2: Consume List through the bound — `for_each` + `fold`

- **Goal**: A generic `for_each` and `fold`, written once against the Iterator bound,
  drain and fold a List — printing `1\n2\n3` and summing 6 — with the bound-dispatch
  question measured and answered.
- **Requirements Covered**: REQ-5, REQ-6, REQ-NFR4
- **Scope**:
  - Modify `lib/core/iterator.sth`: add `for_each['It: Iterator 'T]
    ( 'It['T] [ 'T -- ] -- )` and `fold['It: Iterator 'T 'A]
    ( 'It['T] 'A [ 'A 'T -- 'A ] -- 'A )` — self-recursive, self-call in tail
    position (the `src/ir/driver.rs:1093` transform), `next` dispatched through the
    bound, the `Done` arm dropping only the shell.
  - Modify `tests/phase7b_slice8.rs`: goldens for `for_each` over List (`1\n2\n3`)
    and `0 [ add ] fold` over List (6); the REQ-NFR4 teeth pin (a consumer dispatch arm that
    leaves the `Step` shell undropped fails with the arm-parity error, measured-then-
    pinned byte-exact).
  - Measure REQ-6 first with the minimal bound-dispatch fixture; if
    `poly_cross_call_unsupported_error` (`src/check/poly.rs:4371`) fires, **stop and
    escalate** with the verbatim repro — do not lift the gate in this phase and do
    not reshape `next`.
  - Explicitly out of scope for this phase: Range and anything in `src/parser.rs`,
    `src/ast.rs`, `src/check/`, `src/ir/`; the Option row; per-impl consumer copies.
- **Entry Conditions**: Phase 1 landed — `Iterator`/`Step` exported from
  `lib/core/iterator.sth`, List `next` dispatching at mono call sites
  (`tests/phase7b_slice8.rs` phase-1 goldens green).
- **Exit Criteria / Verifiable Artifacts**:
  - `for_each`/`fold` List goldens pass through the bound (no List-specific word in
    the consumers).
  - REQ-6 answered in writing: either the goldens pass (bound dispatch works) or an
    escalation record exists with the verbatim fence output.
  - Linearity-teeth pin passes; full gate green.
- **Parallelism**: SEQUENTIAL after Phase 1 (the bound needs the trait; consumers
  share `lib/core/iterator.sth` and the new test file with later phases).
- **Relative Effort**: S — two small generic words plus goldens, *provided* the
  expected mechanism holds; the verification discipline is the work.
- **Difficulty**: `hard` — the core interaction (member dispatch through a bound over
  a ctor-headed compound return) is unmeasured; its contingency touches the P7.S3k
  cross-call gate in the poly checker's shared control flow, an ambiguous integration
  point that warrants the stronger model.
- **Open Questions / Blockers**: The REQ-6 verdict itself; the consumer-module import
  surface (the last remaining open question) measured here.

### Phase 3: Ground `Range[i64]` monomorphically — the S2-6 lift

- **Goal**: `impl: Iterator for Range[i64]` declares, its App-headed member grounds
  as a mono instantiation, and `next` over `Range[i64]` dispatches at a plain mono
  call site — while every non-lifted target shape fails byte-exactly as today.
- **Requirements Covered**: REQ-7, REQ-8, REQ-9, REQ-NFR2
- **Scope**:
  - Modify `src/parser.rs:4085` (`parse_impl_target`): intercept at the fold
    (`:4094`) so a fully-applied ctor application with all-concrete type and length
    arguments keeps `PolyType::Generic { args: concrete }` in `ImplTarget.pattern`.
    Do **not** touch `raw_to_poly_type` (`:5655-5729`) — REQ-NFR2.
  - Modify `src/parser.rs:4266` (`parse_impl_member_body`): route Generic-pattern
    targets with all-concrete args through mono grounding — union build
    (`:668` `build_member_var_union`) + `ground_member_poly`'s App dissolve
    (`src/ast.rs:2337-2400`), then the var-free grounded sig converts (via the same
    instantiation machinery the fold uses) to the concrete-path's mono member word
    (`poly: None`, concrete `StackEffect`, `src/parser.rs:4346-4360`).
  - The matcher needs nothing (OQ-1 answered by the code: `match_impl_target_rec`'s
    `Generic` arm `:9235-9319` already compares concrete-arg patterns recursively;
    `find_bound_impl` `:8520` passes `ctx.generics()` through). The check-stage work
    is the continuations, fenced to fully-applied all-concrete targets: (a) a
    lifted-target arm in `resolve_mono_member_call` (`src/check/poly.rs:2233`,
    `is_concrete()` split `:2406`) that checks call-site slots against the mono
    grounded effect and records the dispatch symbol span-keyed,
    `builtin_overloads`-style (`:2434-2437`), instead of the else branch's
    `poly_env` `debug_assert!` (`:2545-2549`); (b) mono-word seeding in the
    obligation/mint path (`impl_mono_seed` `:7927` requires `word.poly = Some`,
    `:7939`; `resolve_user_bound`'s generic-winner arm `:8938-8943`) or a
    concrete-winner-style route for lifted targets — exact mechanism measured first
    (the mono call-site golden is the phase's first artifact); if the obligation/mint
    work cannot land within S8, the recorded option-B fallback applies (REQ-7).
  - Modify `lib/core/iterator.sth`: add `type: Range['T] cur 'T limit 'T ;` and
    `impl: Iterator for Range[i64]` (REQ-9: count-up, `1 add` on i64, advanced Range
    constructed in the `More` arm, iterator dropped inside the `Done` arm).
  - Modify `tests/phase7b_slice8.rs`: (a) Range mono golden — `next` at a plain mono
    call site yields `More` then `Done`; (b) byte-exact pins: `impl: Iterator for
    i64` still raises the S2-6 message (`src/ast.rs:2095` text), an App-headed
    target still raises `impl_target_app_unsupported_error` (`src/parser.rs:902`),
    an applied-var target (`List['L]`) still yields a poly member word; (c) the
    regression pin — `Range[i64]` spelled in an ordinary word signature still folds
    to `Concrete` and shares the mint.
  - Unit tests beside `parse_impl_target`/`parse_impl_member_body` for the new
    target shape and its fences.
  - Explicitly out of scope for this phase: the D5 gate (`src/check/poly.rs:6579`),
    arithmetic traits, phantom parameters, the S6 wall (`:6116-6117`), consumers
    over Range, any `src/ir/` change.
- **Entry Conditions**: Phase 1 landed (Step/Iterator exist — the lift's test
  subject); Phase 2 landed (sequencing discipline: both phases edit
  `lib/core/iterator.sth` and `tests/phase7b_slice8.rs`).
- **Exit Criteria / Verifiable Artifacts**:
  - The Range mono golden passes (`next` dispatches at a mono call site).
  - The three byte-exact fence pins and the mint-sharing regression pin pass.
  - Full gate green with no `src/ir/` diff (REQ-NFR2 verified from the diff).
- **Parallelism**: SEQUENTIAL after Phase 2 (shared files; the dispatch measurement
  also wants the phase-2-stable suite).
- **Relative Effort**: M — two parser touch points plus matcher measurement and a
  new type+impl; the fencing/pinning discipline is the bulk.
- **Difficulty**: `hard` — an impl-target representation change flowing into
  member grounding and dispatch matching; the surrounding fences are load-bearing
  and the shared fold is mint-critical suite-wide.
- **Open Questions / Blockers**: None open — the matcher question is answered by the
  code (OQ-1); the continuation arm and mono-word seeding are this phase's named
  work, with the option-B fallback recorded (REQ-7).

### Phase 4: Range through the bound, IR evidence, and the written record

- **Goal**: The full exit runs — `for_each`/`fold` drain and fold `Range[i64]`
  through the bound (`0 1 2`; sum 3) — and the slice's evidence, ruling, and roadmap
  record are written down with the final gate green.
- **Requirements Covered**: REQ-10, REQ-11, REQ-12, REQ-13, REQ-NFR1
- **Scope**:
  - Modify `tests/phase7b_slice8.rs`: goldens for `for_each` over `Range[i64]`
    (`0\n1\n2`) and `fold` over it (3).
  - Add the REQ-11 IR pin in `tests/phase7b_slice8.rs` via the
    `emit_ssa_with_manifest` capture path (`src/driver.rs:897`);
    `poly_self_tail_call_lowers_to_loop_back_edge` (`src/ir/driver.rs:1093`) is cited
    as the pattern, not the placement, so REQ-NFR2's `src/ir/` diff-empty stands: the
    consuming loop's monomorphized `for_each` is one frame
    whose self-call is a back-edge, and `next` remains a real monomorphized frame.
  - Modify `docs/roadmap/P7b-higher-kinded-types.md` (S8 entry): exhausted-case
    ruling (R1) + fusion evidence written down; exit wording to the landed
    `for_each`/`fold` outcome; implemented-reference link to this spec (house
    convention, cf. the S9/S10 roadmap corrections).
  - Growth-structure re-check per CLAUDE.md at phase exit over every touched file —
    especially `member_shape_is_supported`'s region of `src/parser.rs` (S7 + S8
    edits accumulate there); document the signal check in the roadmap entry.
  - Verify REQ-13 from the diff: `src/check/poly.rs:6116-6117` untouched;
    `tests/phase7b_slice6.rs:410` green.
  - Explicitly out of scope for this phase: any further compiler delta, fusion
    verdicts, adaptor words, S8b work.
- **Entry Conditions**: Phase 2 artifacts (`for_each`/`fold` through the bound over
  List) and Phase 3 artifacts (`Range[i64]` impl dispatching at mono call sites)
  both landed and green.
- **Exit Criteria / Verifiable Artifacts**:
  - Range consumer goldens pass; the full Success Criteria checklist of this spec is
    demonstrable.
  - The IR pin passes and the roadmap entry carries the ruling, evidence, and
    corrections (reviewable by reading the file).
  - Final gate green: `cargo fmt --check && cargo clippy -- -D warnings &&
    cargo test` (3200+ baseline + all new goldens).
- **Parallelism**: SEQUENTIAL after Phases 2 and 3 (needs both; owns the final gate).
- **Relative Effort**: S — goldens for existing consumers plus documentation and
  one IR pin; no new compiler surface.
- **Difficulty**: `standard` — evidence capture and write-downs against
  well-precedented patterns.
- **Open Questions / Blockers**: None identified.

### Parallelism Summary

None: Phase 1 → Phase 2 → Phase 3 → Phase 4, strictly sequential. Phase 2 and 3 share
`lib/core/iterator.sth` and `tests/phase7b_slice8.rs`; Phase 3 also edits the same
parser neighborhood Phase 1 touches; Phase 4 owns the final gate over everything.
(The S7-sequencing note's parallel-edit risk is closed on base `54414cb` — S7 landed
its own arm; this spec's phases avoid recreating the hazard.)

### Effort Summary

- Phase 1: M (standard)
- Phase 2: S (hard)
- Phase 3: M (hard)
- Phase 4: S (standard)
- Total: 2×S + 2×M — roughly 2–3 weeks of sequential work, front-loaded with the
  two verification points (bound dispatch in Phase 2, mono dispatch in Phase 3)
  that retire the slice's named risks.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "member-row gate lift and core Iterator protocol over List", "effort": "M", "difficulty": "standard" },
    { "phase": 2, "focus": "for_each and fold through the Iterator bound over List", "effort": "S", "difficulty": "hard" },
    { "phase": 3, "focus": "S2-6 lift, mono dispatch arming, and Range[i64] impl grounded monomorphically", "effort": "M", "difficulty": "hard" },
    { "phase": 4, "focus": "Range consumers, IR evidence, write-downs and roadmap correction", "effort": "S", "difficulty": "standard" }
  ]
}
```
