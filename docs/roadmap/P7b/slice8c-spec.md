# P7b.S8c — Per-site binding for member signatures with free input type variables

Condensed reference. Implemented on branch `p7b-s8c`; the delivery plan and its
phase scaffolding have been dropped in favour of the commit links below.

## Why

The P7b.S8 integrated review surfaced a pre-existing ICE. A bound-dispatched
trait member whose signature carries a **free input type variable** (not the
trait header variable, not bound by any bound bracket) passed checking and
panicked at IR lowering: `subst_polytype`'s "checked: unification bound every
input type variable" `expect` (`src/ir/driver.rs`), reached through
`concrete_effect` from lowering's R9 instantiation loop. The *check* succeeded;
the panic was a **build-time lowering** panic, so `sooth build` exited 101 and
produced **no binary**.

Dispatch in `resolve_user_bound` (`src/check/poly.rs`) is keyed on the
obligation's `ty` into five routes. Two were broken:

- **Route B — non-CtorImage generic winner** (bound variable in a non-App
  position: plain slot, `&'T`, quotation row, array element, bare-var impl
  target). The P7.S4 mint arm recorded `(member_word, subst)` where `subst` was
  `find_bound_impl`'s impl-target match substitution **alone** — the member
  row's own variables never entered θ, so lowering hit the unbound `expect`.
  **This was the ICE.**
- **Route C — non-CtorImage concrete winner**. Bare symbol, no mint, **no
  site-slot check**. `ground_member_type`'s `Var(_) => target` arm
  (`src/ast.rs`) collapses every free member local to the target type at
  registration, so any operand type flowed through the local slot unchecked —
  a **silent wrong-typing hole** (measured: a `List[i64]`'s head `41` printed
  `41`, a `Bool`'s `True` printed `1`), not a panic.

Routes A (CtorImage per-site composition), D (mono direct member call), and E
(lifted mono ctor-app target) were already healthy and stayed untouched.

## What shipped

### D1 — per-site binding rule (fix direction B)

The Route B mint arm now composes the member instantiation **per site** rather
than being fenced off. It starts `theta` as a **clone** of the impl-target match
subst (extend, never replace), re-grounds the obligation slots through the
caller substitution, and unifies each grounded slot against the member word's
`PolySig` input into `theta`. Every member variable grounds, so the repro that
panicked now **builds and runs**.

Chosen over the two alternatives on measured evidence:

- **Option A (declaration-time plain-slot fence)** was disqualified: the
  committed suite carries green tests riding plain-slot member locals through
  *working* Route B dispatches, including S6's exit-criterion golden
  `Foldable::fold`. A declaration-time fence would outlaw a shape the machinery
  supports.
- **Option C (mint-site fence)** is subsumed: REQ-1's fail-closed tail carries
  C's guarantee inline.

Key sub-rulings, all preserved in the code:

- **Extend, never replace** — a nullary member (e.g. `Monoid::empty`) has zero
  obligation slots, so per-site unification contributes no bindings and the
  header variable's binding survives only because `theta` starts from the match
  subst. Reduces to `theta == subst`, byte-identical to today, no symbol churn.
- **Conflict-diagnostic provenance** — unification runs with empty
  `seeded`/`seeded_len` lists so a conflict against a theta-bound variable
  renders the unseeded `poly_var_conflict_error` wording. The prior binding
  comes from the impl-target **match**, not a user-written instantiation, so the
  seeded-arm "was instantiated at" phrasing would misdescribe provenance.
- **Fail-closed tail (absorbs option C)** — after unification, any member
  variable (inputs **and** outputs) `theta` leaves unbound is a located
  member/variable/site error, never a mint. This is the only fence keeping
  `driver.rs`'s `expect` from firing on this route; REQ-3 forbids touching that
  `expect`.
- **QuotLit fence** — a written quotation literal reaching a plain member slot
  is rejected located at the dispatch site *before* re-grounding, so the marker
  never reaches `apply_subst`'s `unreachable!` `QuotLit` arm (a post-fix hazard
  the re-grounding step would otherwise newly expose).

### D2 — concrete-target hole folded in

The concrete-winner arm gained a site-slot compatibility check, gated on
`imp.target.is_concrete()` so Route E's lifted-mono tenant (S8's `Range[i64]`
golden) stays untouched. A concrete-target member word is registered `poly:
None` with an already-grounded `StackEffect`, so the check is plain `Type`
equality (no `PolySig` to unify): re-ground the slots, compare against the
registered `effect.inputs`, and reuse `trait_member_operand_error` on mismatch.
This closes the silent wrong-typing hole. It does **not** make Route C
polymorphic: a free member local stays pinned to the target type, and the check
rejects operands that disagree.

### Scope held (REQ-3)

`src/check/` only. **Zero `src/ir/` diff** versus the S8b merge base
(`8985d6c`, verified) — `driver.rs`'s `expect` stays as the backstop. No new
trait/impl surface, no `lib/` change.

## Known gaps

- **No type-observing runtime golden.** A generic-target member body cannot
  print a free member local (member rows carry no bounds, so no `Show`-style
  obligation exists to route through; both failure modes were measured). The
  grounding-*correctness* proof is instead the G1a asymmetric-twin monomorph
  symbol capture (`str`/`i64`) plus the Phase 1 theta-content unit assertion.
  The symmetric `i64`/`i64` twins (G1r/G1p, array-element, bare-var-target) are
  grounding-*existence* pins only — they prove the panic→grounded flip but
  cannot discriminate binding provenance.
- **Bare-ctor impl-target cell** grounds under this fix (for `Box` it desugars
  to a one-variable generic pattern) but is **unwitnessed by a dedicated
  golden** — recorded at its measured truth in the roadmap entry.

## Implementation

| Area | Commit | Key symbols / files |
| --- | --- | --- |
| D1 — per-site binding in the Route B mint arm | `8bddbd5` | `compose_member_theta`, `fence_quotation_literal_slot`, fail-closed tail in `resolve_user_bound` (`src/check/poly.rs`); 10 unit tests |
| D2 — concrete-arm site-slot compatibility check | `969cb15` | `check_concrete_member_site_slots` (`src/check/poly.rs`), reuses Phase 1's QuotLit fence; 4 unit tests |
| End-to-end goldens (both routes, cells, witnesses) | `32c2dac` | 10 goldens in `tests/phase7b_slice8.rs` (G1r/G1a/G1p/G2/G3b/G4/G6, QuotLit rejection, array-element + bare-var twins) |
| Roadmap corrigenda + green gate | `11f8747` | `docs/roadmap/P7b-higher-kinded-types.md` (six corrigenda); growth re-check (poly.rs split deferred, 3/5 signals) |

Base for the slice's review history: `5d7087e` (spec). Recon-docs base:
`62a928d`; S8b merge: `8985d6c`. Recon inputs (retained):
[slice8c-brief](./slice8c-brief.md), [slice8c-probes](./slice8c-probes.md),
[slice8c-paper-tests](./slice8c-paper-tests.md).

Final gate at HEAD: `cargo fmt --check && cargo clippy -- -D warnings && cargo
test` green (3357 passed / 0 failed).
