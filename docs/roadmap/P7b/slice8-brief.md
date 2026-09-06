# P7b.S8 brief — linear iterators (HKT as the associated-type substitute)

- Date: 2026-09-05. Base: probes ran at `ae6fdd7`; worktree rebased onto `54414cb`
  the same day (P7b.S7 landed, condensed, round-1 review fixes merged; suite
  green on the new base — fmt, clippy, 2600+ tests, 0 failures).
- Recorded in the roadmap's own S8 entry (`P7b-higher-kinded-types.md`) and in the
  ladder note of 260903 (iterator-without-GATs, linearity-as-protocol).
- Source: probe round **P8, run 260905** — verbatim log at [slice8-probes](./slice8-probes.md)
  (four workers, P8-1..P8-6). The round **refuted this brief's central premise** and
  answered every open question; the "Probe round outcome" section below records the
  verdicts and supersedes the framing wherever the two disagree.

## Problem

Associated types stay out of scope and are not needed: make the iterator itself the
type constructor.

```sth
trait: Iterator['It: * -> *] : next ( 'It['T] -- Option['T] 'It['T] ) ;
```

The "associated type" is just the constructor's own parameter, and linearity *is* the
protocol: `next` consumes the iterator and yields element-plus-remainder — no borrows,
no lifetimes, no `&!` exclusivity puzzles. `drop` is the explicit destructor, so the
exhausted case is exactly the question of *where the final drop happens* (inside
`next`'s `None` arm as one canonical drop site, vs. surfaced to the caller through a
`Step` enum's `Done` arm) — see P8-1.

Grounding-wise the `'It['T]` half is old: a plain variable-headed App row operand is
S2's shipped `Functor.map` shape, the `'It: * -> *` header is S4/S6 machinery, and a
generic consumer dispatching through a shared bound has the S4/S6 precedent
(`shared_bound_poly_word_dispatches_over_the_real_core_option`,
`tests/phase7b_slice4.rs:199`). **The probe round refuted this brief's original
claim that the compiler-side delta for `next` was close to zero**: the row's
`Option['T]` (and any `Step['T 'It['T]]`) half is a *ctor-headed* App, and the
member-row gate rejects every ctor-headed App outright — the trait does not even
declare today. The delta is real and it is gate #1 below; the detailed ledger is
in [slice8-probes](./slice8-probes.md) "Compiler-change ledger".

1. **The S6 construction wall sits in S8's critical path.** slice6-spec "Recorded
   walls": any trait-member body that *constructs* (not merely destructures) its own
   generic struct panics in `poly_bind_construction_arg` on a bare
   `PolyType::Generic` field the Phase 3 `OwnedCell` arm does not cover — witnessed by
   `monoid_for_list_append_construction_wall_is_recorded`, unrelated to recursion, and
   ruled "a future slice's job". S8 needs exactly the constructions the wall blocks:
   - a count-up `Range`'s `next` must construct the advanced `Range` (cur+1) — a
     generic-struct construction inside an impl member body;
   - if `map` over an Iterator is a per-impl member producing `'It['U]` (see P8-5),
     List's `map` reconstructs a `Cons` — the wall's own witness shape.
   `List`'s `next` is unaffected: it only destructures the `Cons` and constructs
   `Option` (enum-ctor construction inside member bodies already works — S6's
   `Functor for Option` map constructs `Some`). **S8 cannot ship its roadmap exit
   without either lifting the wall or shrinking the exit** — a ruling the probe round
   must inform (P8-2), not assume.
2. **The count-up `Range` has no legal impl target today — and the probe round
   named the real fences.** The rejecting site is the **S2-6 concrete impl-target
   fence** (`member_app_concrete_target_error`, `src/ast.rs:2095`, raised from
   `src/parser.rs:4314-4323` once the target folds to `PolyType::Concrete`) — not
   the `member_app_abstract_target_error` twin this brief originally cited; it
   fires identically for `for Range[i64]`, `for Count`, and `for i64`. The other
   candidates died on their own gates: phantom parameters are rejected at the
   typedef *by design* ("a phantom parameter cannot be disambiguated at a call
   site", `src/parser.rs:3049`→`:2512`), and a generic target grounds and
   registers but its body dies at the first aggregate field read (the D5 borrow
   gate, `src/check/poly.rs:6572-6604`) — the arithmetic gap is never reached.
   A plain-word count-up `next` grounds and runs today (P8-3d, `0 1 2`), so the
   Range ruling is: lift S2-6 for fully-applied ctor targets, extend the D5 gate
   plus an arithmetic story for the generic target, or demote Range from the
   goldens (an exit change = user ruling, P8-3).
   **Ruling R2 — DECIDED 260905:** the **S2-6 lift lands in S8** — fully-applied
   ctor applications become legal impl targets, grounded as instantiations
   (`'It` unifies with the ctor head, the member's App args with the target's
   concrete arguments; the member body checks monomorphically). `impl: Iterator
   for Range[i64]` is in the goldens; the roadmap exit keeps "List and Range".
   Scope fences: fully-applied (all-concrete-arg) targets only — mixed/partial
   applications keep today's behavior (S6b-style fencing); the D5 borrow gate and
   any arithmetic-trait surface stay out (the generic-target route is not taken).
   S8 therefore carries two compiler deltas: the member-row `Generic` arm and the
   S2-6 concrete-target grounding. The S7 merge (base `54414cb`) dissolved the
   parallel-edit conflict in the shared parser region.

## Probe questions (round P8; precedes the spec)

- **P8-1 — exhausted case.** `None` with the iterator explicitly dropped inside
  `next`, versus `type: Step['T 'Rest] | Done | More 'T 'Rest ;` instantiated as
  `Step['T 'It['T]]`. The `Step` shape doubles as the parser probe: a nested App as a
  *named* constructor's type argument (`'It['T]` inside `Step[...]`) has no known
  fence (S7's P5 showed named-ctor application lists are unfenced for quotations),
  but nobody has measured it. If both ground, the choice is a user ruling informed by
  which drop site reads better in the consuming loop.
- **P8-2 — wall reproduction.** Reproduce `poly_bind_construction_arg` with (i) a
  single non-recursive `Cons` construction and (ii) a `Range` construction, each
  inside an impl member body, and confirm enum-ctor construction (`Some`) still works
  in the same body. Evidence decides whether the wall fix lands inside S8 or as a
  preceding micro-slice.
- **P8-3 — Range impl target.** Phantom-parameter vs concrete-target lift vs demote
  (candidates (a)/(b)/(c) above). Measure what grounds; user rules what lands.
- **P8-4 — generic consumers.** `for_each` and `fold` written once against a
  `'it: Iterator` bound, looping on `next` over both impls. Does the loop shape
  ground through the shared bound, and does the consuming loop lower to one frame
  (the roadmap's "no frames between")?
- **P8-5 — what "map" means here.** A generic `map` *producing* `'It['U]` is
  impossible against the bound alone (no construction of an opaque constructor — the
  same reason `Applicative.pure` is return-type polymorphic). It can only be a
  per-impl member (List's hits the P8-2 wall) or the exit means a consuming
  `for_each`. Probe evidence + user ruling; the roadmap's "map/fold over an Iterator
  through bounds" is read as the latter until ruled otherwise.
- **P8-6 — fusion evidence.** A small chain of consuming splices: record the IR
  shape (frames, splices). Answer nothing; the fusion question stays a question.

## Probe round outcome (260905 — supersedes the framing above where they disagree)

Every P8-x question is answered; the verbatim evidence is in
[slice8-probes](./slice8-probes.md).

- **P8-1 (exhausted case): measured, both shapes runnable.** With the `Option` row,
  arm parity forces the exhausted iterator to the *caller* — an explicit drop inside
  the exhausted arm is itself the parity breaker, call-site drop is enforced as a
  compile error when forgotten, and a never-moved frame local is reclaimed
  implicitly (the named-slot-locals semantics, not a new wart). With the `Step`
  row, the final drop lives inside `next`'s `Done` arm — one canonical site. Both
  built and ran (`0 1 2`). **Ruling R1 — DECIDED 260905:** the **Step row** becomes
  the protocol: `next ( 'It['T] -- Step['T 'It['T]] )` with `type: Step['T 'Rest] |
  Done | More 'T 'Rest ;` in core, `Done` carrying nothing and the final drop
  inside `next`'s `Done` arm (one canonical site). The Option row (caller-side
  drop, dead iterator surfaced on every `None` path) is the recorded rejected
  alternative. Both shapes need the same member-row fence lift.
- **Round-level finding (refutes the "delta ≈ zero" premise):**
  `member_shape_is_supported` (`src/parser.rs:379`, `Generic` arm `:399-404`)
  rejects every ctor-headed App in a member row — `Option['T]`, `Step['T 'It['T]]`,
  even non-nested `Step['T 'T]`. The `Iterator` trait is undeclarable as spelled;
  the fence is about *ctor applications in member rows*, not nesting. Lifting that
  one arm is S8's first compiler delta and admits both exhausted-case shapes at
  once.
- **P8-2 (wall): refined, narrower than recorded.** The S6 wall is
  **field-shape-specific, not member-specific**: only ctors carrying the
  bare-`Generic` `^Self['T]` self-reference field panic (`poly.rs:6117`) — in any
  poly body, impl member or plain generic word alike. Plain-field ctors construct
  fine inside member bodies (Box2, `Some`, `Nil` all verified end-to-end).
  **Ruling R3 — DECIDED 260905 (absorbed into R4):** the wall fix is **not** in
  S8 — nothing in S8's Step-row `next` shapes needs it; carved out to **S8b**
  with the traitful `List` members (see Scope).
- **P8-3 (Range): every traitful shape rejected today.** Phantom — dead by design;
  concrete-applied target — S2-6 fence (`for Count` too; the predicted kind check
  does not exist); generic target — grounds, then dies at the D5 borrow gate on
  the first field read. **Ruling R2:** lift S2-6 for fully-applied ctor targets,
  or extend the D5 gate plus an arithmetic story for the generic target, or demote
  Range from the goldens (exit change). The plain-word fallback runs today.
- **P8-4 (consumers): mechanism grounds, spelling fenced.** A plain poly
  `list-next` over core List grounds and runs with zero compiler changes (the Nil
  arm must construct the empty remainder — `drop None Nil`; keeping the matched
  Nil hits the variant-escape rule, the bare `drop None` fails arm parity). A
  self-recursive generic `consume` printing `1 2 3` also grounds. Bound dispatch
  is **unmeasurable until the round-level fence lifts**: a poly body cannot call
  another poly word over compound types (`poly.rs:4372-4380`; member dispatch
  through a bound, the `q map` precedent, is expected to work but is pending).
- **P8-5 (map): pinned.** A bound-generic body cannot produce `'It['U]` (located
  error, not a panic) and cannot even `dup` the abstract iterator (conservative
  linearity fence, `poly.rs:10947`). Map is a per-impl member (List's variant is
  wall-blocked, R3) or the exit means consuming `for_each`. **Ruling R4 —
  DECIDED 260905:** the exit reads as consuming **`for_each` + `fold`** through
  the bound; per-impl `map`/`append` and the wall fix are carved out to **S8b**
  (a new slice, decided this date). See Scope.
- **P8-6 (fusion): evidence recorded, no verdict.** The plain-word consuming loop
  lowers to one frame with a back-edge; the per-element work is a real indirect
  call; `list-next` is a real monomorphized frame, *not* spliced; a two-consumer
  chain is two dedicated frames; `.ssa` byte-identical on re-emission. The
  roadmap's "one frame total" is true of the loop, not of loop+next.

## Scope

Per the roadmap entry, adjusted by the decided rulings (R1–R4, 260905):

1. The member-row fence lift: `member_shape_is_supported`'s `Generic` arm
   (`src/parser.rs:399-404`) admits ctor-headed Apps in member rows — the
   prerequisite for the trait itself (admits `Step['T 'It['T]]` and `Option['T]`
   rows alike).
2. A `Step['T 'Rest] | Done | More 'T 'Rest ;` enum in core (module placement
   per spec) — the protocol vehicle per R1.
3. `trait: Iterator['It: * -> *] : next ( 'It['T] -- Step['T 'It['T]] ) ;` with
   impls for `List` (generic target) and `Range[i64]` (R2).
4. The S2-6 concrete-target lift (R2): App-headed members ground monomorphically
   against fully-applied ctor targets; scoped to all-concrete applications
   (mixed/partial keep today's behavior).
5. Generic `for_each`/`fold` through the bound over both impls (R4). `map` is
   **not** in S8.

**Carved out to P7b.S8b (decided 260905):** the S6 construction-wall fix (the
bare-`Generic` `^Self['T]` self-reference-field arm in `poly_bind_construction_arg`,
`src/check/poly.rs:6117`) together with per-impl traitful `List` members (`map`,
`append`) over the Iterator protocol. Nothing in S8's Step-row `next` shapes needs
the wall: plain-field constructions are fine in member bodies; only self-reference
fields panic.

**Sequencing note (parallel-work check of 260905, CONFIRMED by the probe round):**
S8's first compiler delta is the `Generic` arm of `member_shape_is_supported`
(`src/parser.rs:399-404`) — the same gate function whose quotation-row arm S7
lifts (`app_in_member_quotation_row_error`). S7 and S8 are now known to edit the
same function, not merely the same file: order the code merges S7 → S8, or lift
both arms in S7's pass and have S8 consume the lift.
**Post-S7 resolution (base `54414cb`):** S7 landed and merged with its own arm;
S8's `Generic` arm is untouched — re-verified on the new base (`bind` now
declares; the `Iterator` row still fences). S7's lift also **reworded the fence
message** (now "...plus a trait-var-headed application, in a plain slot or a
quotation row..."); S8's spec pins diagnostics against the new text, not the
probe log's.

## Explicitly out of scope

- Associated types / GATs — the whole point of the slice is that they are not needed.
- Borrow-based iteration (`&!`, lifetimes, exclusivity) — linearity replaces it.
- Lazy/streaming iterators and an adaptor library (`zip`/`take`/`rev`/...) — no free
  library work; the trait is the dogfood, not a stdlib slice.
- Fusion as an answered question — evidence recorded (P8-6), ruling deferred.
- Iterator for `array` — S6b territory (array-as-constructor), not here.
- Any change to `Option`'s shape or `core::option`'s surface — consumed as-is.

## Dogfood / exit

Per the rulings: `next`/`for_each`/`fold` through an Iterator bound over `List`
and `Range[i64]` goldens (Step-row protocol); a consuming loop runs with one
frame total (P8-6 evidence recorded: true of the loop, not of loop+`next`); the
exhausted-case ruling and the fusion evidence from a small chain are written
down. The roadmap entry's "map/fold" wording reads as `for_each`/`fold` (R4).
