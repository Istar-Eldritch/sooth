# P7b.S8c brief — located fence for member signatures with unbindable free type variables (recon round)

Scope input for the S8c spec. Produced by a recon round against the clean tree
(worktree `p7b-s8c`, **rebased onto `main` `445a74e` before probing** — the
worktree had been cut before the S6/S6c/S7/S8/S8b merges and carried none of
the S8 machinery the repro lives in), then adjudicated by a probe round
(verbatim log, fixtures, and verdicts:
[slice8c-probes](./slice8c-probes.md)) and a paper-test round (golden designs:
[slice8c-paper-tests](./slice8c-paper-tests.md)). **Round 2** re-measured
everything with five `prober` workers after the tree gained the S8b merge
(base `8985d6c`, round-2 HEAD `62a928d`): every round-1 measurement
re-verified byte-identical, two new panic cells measured, and two round-1
claims **refuted** (D1 option A's blast radius; the concrete hole's
observability) — the round-2 amendments are folded into F1-F5/D1/D2 below.
Repo untouched throughout. Baseline at round-2 HEAD: `cargo fmt --check &&
cargo clippy -- -D warnings && cargo test` **green — 89 binaries, 3334
passing, 0 failed**.

S8c is the P7b.S8 integrated review's pre-existing ICE: a bound-dispatched
trait member whose signature carries a free INPUT type variable passes
checking and panics at IR instantiation (`subst_polytype`'s unification
`expect`, `src/ir/driver.rs:579`). Size `S`, scope `src/check/` only, no
`src/ir/` change. **The probe round has rewritten the mechanism story**: the
roadmap's shape is one cell of a five-route dispatch taxonomy, the panic is a
build-time lowering panic (not a "run/IR time" panic of a built binary), and
the same obligation loop hides an adjacent *silent* hole on the concrete-target
route. Two wording corrigenda and one scope decision flow to the spec.

## What the slice must close (roadmap exit, as amended by evidence)

1. **The repro, panicked→located** — `plainslot.sth` (probes P1) panics today
   at `driver.rs:579:14`, `sooth build` exit 101, no binary. Post-fix: located
   or grounded (the D1 decision), never a panic. Golden G1/G1r.
2. **The mono call-site route stays healthy** — the same trait called directly
   works today (probes P2) and is a positive pin, not a fence casualty. G2.
3. **The quotation-row twin** — a member local in a quotation-row input
   reaches the same hole once the caller satisfies the pre-existing
   "supply it explicitly" rule with an explicit instantiation (probes P4/P5).
   P4's caller rule is not this slice's fence; P5 is the same bug in a
   different position. G3a/G3b.
4. **Route A survives whichever direction wins** — the App-headed/CtorImage
   route grounds plain-slot member locals per site today (probes P6/P7/P8;
   this is what every shipped S6/S7/S8 dogfood rides). A declaration-time
   fence keyed on "member local in a plain slot" would outlaw a shape the
   machinery supports; G4 is the canary, G5 the checklist item.

## What the recon round established (mechanism, probes verdict "Mechanism")

- **F1 — the route decides.** `resolve_user_bound` (`src/check/poly.rs:8981`
  at the S8b base; byte-identical across the merge, only a +251-line offset)
  branches on the obligation's `ty`: the CtorImage arm (App-headed operand,
  S2-3) composes per site at instantiation — re-grounds `ob.slots` through
  `caller_subst`, unifies the member word's signature (`unify_poly_input`),
  mints θ_call — so member locals ground (P3/P6/P7/P8). No shipped `lib/`
  trait reaches the mint arm (w3r arm attribution: Show/Write/Ord → Route C,
  Iterator/List → Route A, Iterator/Range[i64] → Route E; Functor/Foldable/
  Monoid exist only as test-fixture traits). The P7.S4 mint arm
  (now `poly.rs:9315-9323`, generic winner, non-CtorImage) records
  `(member_word, subst)` under `find_bound_impl`'s impl-target match
  substitution **alone** — the member row's own variables never enter θ, and
  lowering's `concrete_effect` (`src/ir/driver.rs:555`, called from the R9
  instantiation loop at `driver.rs:327`) hits the unbound `expect` at
  `driver.rs:579` (P1/P5/P9/P10, backtrace P13, zero drift at the S8b base).
  Round 2 widened the cell: the hole also swallows an **array-element**
  member local (`array['U 2]`, admitted by the Array arm) and the
  **bare-var catch-all** target (`impl: Odd for 'T`) — all three
  generic-target spellings are one panic cell.
- **F2 — the hole is admission-wide, position-wide.**
  `member_shape_is_supported`'s `PolyType::Var(_) => true` arm
  (`src/parser.rs:391`) admits member locals in any non-App position
  unconditionally; only App heads are restricted to the header variable. The
  panic shape needs no exotic row: a bare header slot plus one local (P9) or
  a Ref over the header plus one local (P1) — either impl-target spelling
  (`for Box['T]`, bare `for Box`, P10) — suffices.
- **F3 — output-position locals are already fenced.** No body can produce an
  unbound variable's value; the impl member body check rejects located
  ("body leaves `i64`, but the declared outputs are `'U`", probes P14). The
  slice's fence, whichever direction, only has input/ref/quotation positions
  to worry about.
- **F4 — the concrete-target route is a silent hole, not a panic — and it is
  dispatch-specific and exploit-confirmed (round 2).**
  `ground_member_type`'s `PolyType::Var(_) => target` (`src/ast.rs:2241`;
  round 1 cited `2239`) collapses every member local to the target type at
  registration (`parse_impl_member_body` → `ground_member_type` at
  `parser.rs:4589`, mono `WordDef { poly: None }`), so the member word emits
  as one mono function typed wholly at the target (`nm`:
  `odd.::Odd.::0.::i64`), and the non-CtorImage concrete-winner arm keeps the
  bare symbol **without any site-slot compatibility check**
  (`poly.rs:9324-9331`; the only slot checks live in the CtorImage branch).
  The direct mono call of the same member **is** checked (Route D's
  `trait_member_operand_error`: "expects `i64`, found `Bool` in operand slot
  1") — so the hole is bound-dispatch-specific. And it produces **unambiguous
  wrong output**: a member body computing on the local slot prints a `Bool`'s
  discriminant `+1` or a `List[i64]`'s head word `+1` — value varies with
  what flows through the slot; no type-respecting execution explains it.
  Same obligation loop as the ICE, different failure class (silent wrong
  typing). D2 decides: fold the compatibility check into S8c or carve it
  out; round 2 raises the urgency.
- **F5 — the checker/IR contract mismatch, stated once.** The IR `expect`'s
  invariant ("checked: unification bound every input type variable") is true
  for every instantiation minted by call-site unification (Routes A and D)
  and false **by construction** for Route B's match-only mint. The checker
  never enforced that a member row's variables are all determined by the
  impl-target pattern; the invariant's enforcement point simply never
  existed.

## Corrigenda for the roadmap entry (`P7b-higher-kinded-types.md`, S8c paragraph)

1. "the build succeeds and the PANIC fires at run/IR time" → the *check*
   succeeds; the panic fires during **lowering at build time** (`sooth build`
   exits 101, produces no binary; probes P1/P13). The paragraph is corrected
   at slice exit with the measured wording.
2. The repro paragraph reads as if the shape is the whole surface → it is one
   cell of the route taxonomy (probes doc, summary table); the quotation-row
   twin (P5), the array-element and bare-var-target cells (round 2), and the
   concrete-target hole (P12, round 2: exploit-confirmed) are recorded
   alongside.
3. "pre-existing; reproducible at the S8 base `86ca5eb`" → carried forward as
   recorded (verified by the S8 integrated review; not re-verified this round
   — the rebased base `445a74e` includes S8, and the panic reproduces there;
   round 2 re-verified at the S8b base `62a928d`, byte-identical).
4. Citation drift at the S8b base: the Route B mint arm is now
   `src/check/poly.rs:9315-9323` (`resolve_user_bound` at `8981`);
   `ground_member_type`'s Var arm is `src/ast.rs:2241` (round 1 said 2239);
   `driver.rs:327/555/579` and `parser.rs:391` still exact.

## D1 — the fix direction (the spec's decision; the roadmap delegates it here)

Measured consequences, from the probes:

- **Option A — declaration-time located fence** (reject a member type variable
  that neither the header nor any bound binds, where it is declared).
  Simplest diagnostic (the declaration is the site); makes G1/G3a red→green.
  **Round 2 refuted the round-1 cost claim**: the fence is not just a
  capability regression with "zero victims" — the committed suite carries
  positive green tests riding plain-slot member locals through **Route B
  dispatches that work** (round 2's new measurement: `Functor::pick`
  `tests/phase7b_slice2.rs:146`, `Take::take` `tests/phase7b_slice3.rs:426`,
  `Foldable::fold` `tests/phase7b_slice6.rs:337/372/469` — the last being
  **S6's exit-criterion dogfood golden**), so a declaration-time plain-slot
  fence breaks **≥7 green test functions in 5 files** plus two src unit twins
  (`declarations.rs:3860`, `poly.rs:22301`); only the two `pick` rows are
  spareable, by placing the fence at check time ordered after S2-15.a's own
  diagnostic. Rows 2-5 break under **any** fence placement. `lib/` itself
  stays clean, but the suite is the spec's regression surface, not `lib/`.
  The fence must also cover array-element positions (round 2's new cell),
  not just top-level plain slots.
- **Option B — the binding rule** (ground free member variables per site, like
  the doc's `'B` — Bifunctor's row locals — already ground through Route A's
  composition). Extends the CtorImage arm's per-site unification to the
  P7.S4 mint arm: re-ground `ob.slots` through `caller_subst`, unify the
  member word's signature, mint that site's θ (sorted per the P7.S3t
  invariant). Makes G1r/G3b green; the repro *works* instead of being
  rejected. Costs: two sites can bind a local differently → two monomorphs
  (existing dedup handles it). **Symbol churn: measured nil (w3r)** — no
  shipped dispatch reaches the mint arm at all (Show/Write/Ord → Route C,
  Iterator/List → Route A, Iterator/Range[i64] → Route E; the 49-symbol
  inventory baseline contains no mint-arm dispatch), the len-pair
  encounter-order hazard has zero victims (no `Len::Var` in the shipped
  dispatch surface), and where both substitutions exist the binding sets
  agree, so a per-site symbol is byte-identical to today's. The predicted
  post-fix repro monomorph is
  `sooth_mono_odd_Odd_0_Box__T0___m0__t0_i64_t1_i64` (member word registered
  as `odd;Odd;0;Box['T0]__m0`; θ binds the target var and the local, both to
  `i64`). One shape note: a consumer row that spells its slots in a different
  order than the member row fails located at the pre-existing operand check
  before any dispatch (w3r Q4a) — option B changes nothing about that fence.
  It leaves Route C's missing compatibility check untouched unless D2 folds
  it in (the same unification is exactly that check).
- **Option C — the mint-site fence** (located error in the non-CtorImage
  mint arm when, after the match substitution, a member variable remains
  unbound — the IR `expect`'s predicate, checked instead of trusted, at the
  member call site). Narrowest behaviour change: only shapes that panic
  today change (to located errors); Route A untouched by construction —
  which round 2 makes load-bearing, since Route A's plain-slot locals are
  now known to be **pinned by green goldens** (S6's `fold` dogfood), not
  merely permitted; G1/G3a green with call-site (not declaration) wording.
  Cost: it fences a shape Route A supports, but keyed on the *actual*
  failing condition (route + unbound var) rather than the row's static
  shape — no working program changes.

The probe evidence after round 2 leans clearly toward **B** (the hole is a
missing half of machinery that exists one arm over; B subsumes C's safety when
D2 folds the concrete arm in; and A — the roadmap's first-named option — is
now measured to break S6's own exit-criterion golden, which effectively
disqualifies the parse-time placement and leaves check-time A as a narrower,
ordering-fragile variant), with **C** as the conservative fallback; the spec
owns the call.

## D2 — the concrete-target hole (fold in or carve out)

`F4`'s silent no-site-check dispatch is adjacent (same obligation loop, the
arm directly below the mint arm) but is a different bug class — silent wrong
typing, no panic, no diagnostic. **Round 2 raised the urgency**: the hole is
bound-dispatch-specific (the direct call is checked, the bound dispatch is
not) and produces unambiguous wrong output (a `Bool`'s discriminant `+1` or a
`List[i64]`'s head word `+1` printed through a member typed at the target).
Folding it in costs one slot-compatibility unification in the concrete-winner
arm and is exactly option B's machinery reused; Route D's
`trait_member_operand_error` text is the ready diagnostic template (w2).
Carving it out keeps S8c a pure panicked→located slice (roadmap size `S`)
and leaves the hole recorded for its own slice. The paper doc's G6 is
written to move either way.

## Non-goals (carried from the roadmap scope, unchanged by the recon)

- No `src/ir/` change: `driver.rs:579`'s `expect` remains the backstop.
- P4's caller-side rule ("output variable that no input binds / supply it
  explicitly") is pre-existing behaviour, not this slice's diagnostic; it
  stays byte-identical.
- No new trait/impl surface, no lib changes; `lib/` traits are only the
  regression net (G5).
