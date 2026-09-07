# P7b.S8c brief — located fence for member signatures with unbindable free type variables (recon round)

Scope input for the S8c spec. Produced by a recon round against the clean tree
(worktree `p7b-s8c`, **rebased onto `main` `445a74e` before probing** — the
worktree had been cut before the S6/S6c/S7/S8/S8b merges and carried none of
the S8 machinery the repro lives in), then adjudicated by a probe round
(verbatim log, fixtures, and verdicts:
[slice8c-probes](./slice8c-probes.md)) and a paper-test round (golden designs:
[slice8c-paper-tests](./slice8c-paper-tests.md)). Repo untouched throughout.
Baseline at HEAD: `cargo fmt --check && cargo clippy -- -D warnings && cargo
test` **green — 88 binaries, 3293 passing, 0 failed**.

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

- **F1 — the route decides.** `resolve_user_bound` (`src/check/poly.rs`)
  branches on the obligation's `ty`: the CtorImage arm (App-headed operand,
  S2-3) composes per site at instantiation — re-grounds `ob.slots` through
  `caller_subst`, unifies the member word's signature (`unify_poly_input`),
  mints θ_call — so member locals ground (P3/P6/P7/P8). The P7.S4 mint arm
  (`poly.rs:9065-9071`, generic winner, non-CtorImage) records
  `(member_word, subst)` under `find_bound_impl`'s impl-target match
  substitution **alone**; the member row's own variables never enter θ, and
  lowering's `concrete_effect` (`src/ir/driver.rs:555`, called from the R9
  instantiation loop at `driver.rs:327`) hits the unbound `expect` at
  `driver.rs:579` (P1/P5/P9/P10, backtrace P13).
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
- **F4 — the concrete-target route is a silent hole, not a panic.**
  `ground_member_type`'s `PolyType::Var(_) => target` (`src/ast.rs:2239`)
  collapses every member local to the target type at registration, so the
  member word emits as one mono function typed wholly at the target
  (`nm`: `odd.::Odd.::0.::i64`), and the non-CtorImage concrete-winner arm
  keeps the bare symbol **without any site-slot compatibility check** — a
  `List[i64]` flows through the local's slot unchecked (probes P11/P12).
  Same obligation loop, different failure class (silent wrong typing). D2
  decides: fold the compatibility check into S8c or carve it out.
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
   twin (P5) and the concrete-target hole (P12) are recorded alongside.
3. "pre-existing; reproducible at the S8 base `86ca5eb`" → carried forward as
   recorded (verified by the S8 integrated review; not re-verified this round
   — the rebased base `445a74e` includes S8, and the panic reproduces there).

## D1 — the fix direction (the spec's decision; the roadmap delegates it here)

Measured consequences, from the probes:

- **Option A — declaration-time located fence** (reject a member type variable
  that neither the header nor any bound binds, where it is declared).
  Simplest diagnostic (the declaration is the site); makes G1/G3a red→green.
  Cost: it outlaws **Route A's working shapes** — mixed3's plain-slot local
  `'V` (P8) and any future HKT member that wants one — a capability
  regression with zero shipped-lib victims today (no `lib/` trait member has
  a plain-slot local; grepped) but a real narrowing of the surface S2-3
  opened. The fence must live in the member-signature admission path
  (`member_shape_is_supported`'s Var arm or its caller), and G4 is the
  canary proving it did not overreach into App-arg positions.
- **Option B — the binding rule** (ground free member variables per site, like
  the doc's `'B` — Bifunctor's row locals — already ground through Route A's
  composition). Extends the CtorImage arm's per-site unification to the
  P7.S4 mint arm: re-ground `ob.slots` through `caller_subst`, unify the
  member word's signature, mint that site's θ (sorted per the P7.S3t
  invariant). Makes G1r/G3b green; the repro *works* instead of being
  rejected. Costs: two sites can bind a local differently → two monomorphs
  (existing dedup handles it); symbol-order churn risk for shipped traits
  (expected nil — their locals are all target-pattern-determined — measured
  by G5's checklist); and it leaves Route C's missing compatibility check
  untouched unless D2 folds it in (the same unification is exactly that
  check).
- **Option C — the mint-site fence** (located error in the non-CtorImage
  mint arm when, after the match substitution, a member variable remains
  unbound — the IR `expect`'s predicate, checked instead of trusted, at the
  member call site). Narrowest behaviour change: only shapes that panic
  today change (to located errors); Route A untouched by construction;
  G1/G3a green with call-site (not declaration) wording. Cost: it fences a
  shape Route A supports, but keyed on the *actual* failing condition
  (route + unbound var) rather than the row's static shape — no working
  program changes.

The probe evidence leans toward B (the hole is a missing half of machinery
that exists one arm over, and B subsumes C's safety when D2 folds the
concrete arm in) with C as the conservative fallback; the spec owns the call.

## D2 — the concrete-target hole (fold in or carve out)

`F4`'s silent no-site-check dispatch is adjacent (same obligation loop, the
arm directly below the mint arm) but is a different bug class — silent wrong
typing, no panic, no diagnostic. Folding it in costs one slot-compatibility
unification in the concrete-winner arm and is exactly option B's machinery
reused; carving it out keeps S8c a pure panicked→located slice (roadmap
size `S`) and leaves the hole recorded for its own slice. The paper doc's G6
is written to move either way.

## Non-goals (carried from the roadmap scope, unchanged by the recon)

- No `src/ir/` change: `driver.rs:579`'s `expect` remains the backstop.
- P4's caller-side rule ("output variable that no input binds / supply it
  explicitly") is pre-existing behaviour, not this slice's diagnostic; it
  stays byte-identical.
- No new trait/impl surface, no lib changes; `lib/` traits are only the
  regression net (G5).
