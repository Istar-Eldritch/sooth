# Phase 4 Slice 7b: capturing closures (brief)

7a shipped a quotation as a `(code, env)` runtime value, but only for one that captures
nothing: a capturing literal reaching any of 7a's four materialization boundaries (a struct
field, an array element, a word output, a differing-arm branch join) is rejected outright.
Verified against the built compiler:

```
type: Holder q [ i64 -- i64 ] ;
: main ( -- ) 10 | x | [ x + ] Holder drop ;
=> error: a capturing quotation cannot be stored (capturing closures are slice 7b) (line 4)
```

This slice is what makes that legal: an env that actually holds something, and a checker that
tracks how long the something it holds must stay alive.

## Recon: measured against the built compiler and read against the current checker

**1. Capture is already free everywhere a quotation is inlined — this slice does not touch
that path.** Verified three ways: a capturing literal spliced directly at `call` runs today
(`4 [ x + ] call .` prints `14`); through `times` (`0 3 [ | i | x + ] times .` prints `30`);
and passed as an argument to an ordinary user word declaring a quotation parameter — which is
a combinator by definition (6a) and therefore force-inlined, never a real call — also runs
unmodified (`: apply ( i64 [ i64 -- i64 ] -- i64 ) call ; ... 4 [ x + ] apply .` prints `14`).
Every quotation-parameter position in the language is inlined (D2, 6a); the *only* places a
quotation's identity can erase are 7a's four materialization boundaries. This slice's entire
surface is those four boundaries, plus `call`'s new by-reference forms (recon 9). Nothing about
splicing changes.

**2. The capture-set analysis already exists — built by 6f, not open as the ROADMAP currently
states.** The 7 entry's 7b paragraph says "carries the capture-set analysis (7a needs only a
*predicate*... and none exists today either way)". That was true when 7a's brief was written;
6f built it in the meantime to fix its own liveness gap. `capture_names`/`capture_names_into`
(`src/check.rs:777-834`) compute a quotation literal's free-name set once, at intern time,
recursing into nested quotations and both `if` arms; `Provenance::quotation_captures`
(`src/check.rs:544`) caches it by `QuotId`. This is a real set, not a boolean — the ROADMAP
sentence is corrected below rather than left to mislead the spec.

**3. 6f's reachability machinery is exactly what 7b needs, and exactly where it stops.**
`capture_alive_names` (`src/check.rs:1054`) fixed-point-walks the stack and scope: for every
slot or binding whose `quot` is `Some(QuotRef::Known(id))` and is itself alive (on the stack,
unconditionally reachable; bound, reachable iff `!live.dead(name, at)`), it unions in
`quotation_captures(id)`, repeating until nothing new is added — so a name captured by a
quotation that is itself captured by another live quotation is still counted. `live_derivs`
(`:1097`) folds this into the ordinary derivation-liveness scan a plain reference already gets.
This is precisely "point Slice 6's escape checking at a new carrier": the carrier already
exists, the walk already reaches it, for exactly one reason it was built for a different
problem — keeping an ordinary borrow alive while a *still-`Known`* quotation might read it
again. The walk's `if let Some(QuotRef::Known(id)) = slot.quot` guard is the whole of its reach:
the moment `quot` becomes `None`, the value drops out of consideration completely.

**4. `QuotRef` is single-variant by an explicit, dated design note that stops holding the
moment a join can merge two different capturing literals.** `src/check.rs:85-91`: "A single
variant: two *different* quotations at a branch join are rejected at the join (R7), so no
poisoned/merged marker is ever carried." True and load-bearing today — 7a's join only ever
erases *non-capturing* literals into a real `Type::Quotation` value with `quot: None`, and a
non-capturing literal has no capture set to lose. The moment 7b allows two *different*
capturing literals to join — unifying effects, both of them capturing, neither identifiable
afterwards — there is no representation left standing for "this value came from one of these N
literals, so its capture set is the union of theirs." Recon 3's walk cannot see through `None`,
and `None` is where every erased capturing quotation lands under 7a's boundary design unless
something changes.

**5. The env is not a stub today — it is a hardcoded constant, unconditionally, in shipped
code.** `materialize_quot_value` (`src/ir.rs:4170-4191`): `let env = self.fresh_value(IrType::Ptr);
self.push_instr(Instr::Const(env, 0));`, every time, no branch, no field read anywhere. There is
no partial env mechanism to extend — 7b builds the env from nothing, not from a stub.

**6. Slice 6's structural escape check cannot see inside a quotation's env, by the same
opacity that makes the representation useful.** `contains_reference` (`src/check.rs:285-307`)
recurses into `Struct`/`Enum`/`Array` field types hunting `Type::Ref`, with a wildcard `_ =>
false` — so `Type::Quotation` (and `Type::OwnedCell`) are invisible to it today, exactly as
`IrType::Code`'s deliberate opacity intends. `fill` and `^` are its only two call sites
(`src/check.rs:8437`/`:8497`), both catching a reference smuggled in as a `Copy` value past the
declaration-site sweep. Whatever soundness 7b needs cannot come from teaching this predicate to
look inside a quotation's env — it has to come from the flow-sensitive, value-identity-keyed
walk in recon 3, because the type-level check is structurally blind to what an opaque code/env
pair might be holding.

**7. `^`'s own checker code already rejects a reference-containing payload, unconditionally,
today.** `check_owned_cell_word`'s `"^"` arm (`src/check.rs:8497`) calls
`contains_reference(payload, ...)` before interning and rejects on `true`. If `^Env` (the
ROADMAP's own phrase for an upward closure) means "an owned cell over a synthesized
capture-struct with `&T` fields," that construction is rejected by *existing*, general-purpose
code today, not by anything 7a or 7b adds. Any spec that wants `^Env` to reach `intern_owned_cell_type`
needs an explicit, scoped carve-out here — not a silent bypass of a check written for a
different purpose that happens to also fire on this one.

**8. The synthesized-struct precedent (`intern_bundle_struct`, `src/ast.rs:433`) has never
carried a reference-typed field**, and per recon 6/7, whichever internal shape 7b picks for a
capture bundle has direct, opposite-facing consequences: a real `StructDecl` with a `Type::Ref`
field is *visible* to `contains_reference` at every position that struct type reaches (an
ordinary struct containing it, an array of it, `fill` over it) even though the *quotation's own*
env is invisible to the same check when the capture rides opaquely behind `IrType::Code`/`Ptr`
instead. The representation choice is also a choice about which existing guard rail applies.

**9. `call` pops its operand by value, unconditionally, today; there is no reference-mode call
anywhere in the grammar or checker.** `src/check.rs:6716-6718`. The "Fn/FnMut/FnOnce-equivalent
split... through `&q`, `&!q`, and by value" the ROADMAP names is new surface syntax and new
checking from nothing, the same way `@`/`!`/`+!` are new relative to a bare local read — not a
relaxation of an existing case.

**10. The old R12 wording names this slice as a future destination that, once this slice
ships, no longer is one.** `capturing_quotation_error` (`src/check.rs`, 7a) reads "a capturing
quotation cannot {boundary} (**capturing closures are slice 7b**)". Once 7b exists, every
program this message currently rejects either becomes legal (a non-escaping capture, live
through its use) or gets a *different* located rejection (an escaping one past its last use, or
past its owning frame). The old wording has no correct audience left to address once this slice
ships — it is not a message 7b's own diagnostics reuse or extend, it is one 7b's existence
retires.

## Decisions

- **D1. The four materialization boundaries are unchanged; only the admission rule at them
  changes.** 7a's struct-field/array-element/word-output/branch-join boundaries stay exactly
  where they are (`check.rs`, cited in the 7a brief). This slice widens what may cross them —
  a capturing literal, provided its captures are still trackable — rather than adding a fifth
  boundary or moving the existing four.
- **D2. Direct-splice capture needs no new checking.** Recon 1 is exhaustive: every
  quotation-parameter position inlines, so nothing about `call`, `times`, or a combinator's own
  quotation argument changes. The entire new surface is materialization plus the new call
  modes (Q3).
- **D3. Reuse `quotation_captures`/`capture_names` as the capture-set source; no new
  computation.** Recon 2. Whatever the spec needs the env's fields to be, it enumerates them
  from the existing cached set, keyed by `QuotId`, not from a new analysis pass.
- **D4. The env holds a reference, never a snapshot, in every case, including an escaping
  one.** This is the ROADMAP's own explicit position and 7a's D3/recon (a snapshot silently
  breaks `map`'s captured-array mutation pattern in `lib/combinators.sth`); nothing found in
  this recon weakens it. Whatever an "upward" closure turns out to mean (Q2), it is not
  "copy the capture in and stop needing a reference."

## Open questions for the spec

These are genuine forks, not gaps in the recon — I have a reading of the evidence for each but
not a confident single answer, and getting one wrong is expensive precisely because it is a
representation decision (7a's own D5 is what kept 7b additive; the wrong call here could cost
7a's uniformity or 7b's own follow-on).

- **Q1. How does an erased quotation's capture set survive materialization, given `QuotRef`'s
  single-variant design (recon 4) and `capture_alive_names`'s hard dependency on `Known`
  (recon 3)?** Two directions, not necessarily exhaustive: (a) widen `QuotRef` with a second
  variant carrying a capture-set handle (e.g. `Materialized(CaptureSetId)`, the join's union of
  both arms' sets when they differ) so recon 3's walk gets a second case instead of going blind
  at `None`; (b) restrict materialization of a capturing literal to shapes where 6f's *existing*
  liveness already keeps every captured name alive on its own merits (no new tracking needed,
  at the cost of rejecting some upward escapes that (a) would allow). (a) is more capable and
  is what the exit criteria's "called while that capture is still live" bullet implies is
  needed in general; (b) is smaller and might be enough for a first cut, deferring the general
  case the way 7a deferred capture at all.

- **Q2. What does `^Env` actually own?** Two readings of the ROADMAP's own words, which read as
  compatible until recon 6/7 forces the question: (i) the env struct itself is heap-allocated
  (an `OwnedCellId` over a synthesized reference-carrying capture struct), single-owner so
  dropping the closure disposes it — this needs the recon 7 carve-out in `contains_reference`'s
  `^`-site check, explicitly scoped to this one synthesized shape, not a general relaxation; or
  (ii) a capture that needs to escape upward must itself already be `^T`-owned, and "capture by
  reference" for that case means borrowing *through* the existing heap pointer, whose address
  is stable regardless of which frame currently holds it — sidestepping frame-liveness
  reasoning entirely in favor of the *existing* linear move-tracking on the `^T` handle, at the
  cost of restricting what an upward closure may capture (only `^T`-owned data, not an ordinary
  stack-resident aggregate). (ii) is smaller, reuses more, and costs a real restriction; (i) is
  the more literal reading of "upward closures on `^Env`" but is new machinery on top of new
  machinery. Recon 7's existing rejection has to be dealt with explicitly either way, since (ii)
  still needs to explain why capturing a `^T` doesn't just move it (must be a *borrow* of the
  cell, not a consumption).

- **Q3. What are the exact semantics of `call` through `&q` and `&!q`, and what does each mode
  do to the env?** By-value `call` today consumes its operand (recon 9); does it now also mean
  "this is the last call, the captured references' obligations end here"? Does `&q`/`&!q`
  leave the closure callable again — required for a closure invoked repeatedly from inside
  `each`/`times`/a dispatch table, which is the actually-motivating use case per the roadmap's
  own `cond [ fast ] [ slow ] if call`/dispatch-table framing — and if so, does repeated calling
  through `&!q` let the closure mutate its own captures across calls (an `FnMut` shape), or is
  mutation still routed entirely through what the captured reference itself points at (no
  separate closure-local mutable state at all, keeping the closure itself immutable and letting
  `&!q` mean only "callable without consuming," not "callable with internal state change")?

- **Q4. Does joining two different capturing literals need the union of both arms' capture
  sets, or does it need something narrower?** If Q1 picks direction (a), the join produces one
  value that might be either literal; keeping every name either might read alive for as long as
  the joined value survives is conservative but sound. Does this conservatism compound if the
  joined value itself later flows into a further join or store (widening the tracked set
  further each time), and if so, is that an acceptable cost or does it need a cap (e.g., "a
  capturing quotation may cross at most one join before its capture set is frozen")?

- **Q5. Does the old R12 wording get retired, and with what replacement vocabulary?** Recon 10:
  "capturing closures are slice 7b" has no correct audience once 7b ships. The exit criteria
  imply at least two new diagnostics (past-last-use, past-owning-frame) with their own located
  wording; decide whether they reuse `capturing_quotation_error`'s shape with a different tail
  clause or are new functions entirely, and whether any *existing* 7a golden asserting the old
  exact string needs to move to a still-rejected shape (a capture that is provably never live
  at its use) rather than simply being deleted.

## Out of scope

- **Opt-in RC (`Rc`/`Arc`-equivalent).** Explicitly deferred to Phase 6 (Phase 3 Slice 6's own
  exit note); not reachable from here regardless of how Q2 resolves.
- **Any change to non-capturing quotation behaviour.** 7a's splice/materialize/indirect-call
  story for a quotation that captures nothing is unchanged; this slice only widens the
  admission rule at the same four boundaries (D1).
- **Any change to combinator inlining.** `each`/`map`/`fold`/`while`/`times` stay force-inlined
  (6a's D2); recon 1 confirms captures are already free there, so there is nothing to change.
- **The clause-body rejection** (`check.rs:1251`). Unrelated, as 7a's brief also noted: it is
  about splicing a callee, not representing a captured value.

## Exit

A closure capturing an aggregate, called while that capture is still live, compiles and
observes the same values the spliced form does; one captured past its last use — or, for an
upward closure, past its owning frame — is rejected with a located error naming the capture;
dropping a linear-capturing closure disposes its captures.
