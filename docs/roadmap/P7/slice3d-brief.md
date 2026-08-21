# Phase 7 Slice 3d: rowless quotation-consumer splice (brief)

A quotation literal in a **non-inline** polymorphic body has exactly one legal consumer
today: an enum eliminator (P7.S3b). Every other use — `call`, or passing the literal as
an argument to another word — is a located rejection, regardless of whether the consumer
actually needs row unification against the abstract stack. This slice is the cheap tier
P7.S3b-follow already assumes as a dependency: splice a **fully concrete (rowless)**
quotation consumer through the poly walk, with no row (`..a`/`..b`) machinery.

Probe-verified at HEAD, two distinct rejections, not one:

```sooth
: caller ( 'T: Copy 'T -- 'T ) [ ] call ;
```

```text
error: `call` on a quotation in the polymorphic body of `caller` (line 1) is not yet supported
  only an enum eliminator consumes a quotation in a generic body today (P7.S3b-follow)
```

```sooth
: helper inline ( 'T ~[ 'T -- 'T ] -- 'T ) call ;
: caller ( 'T -- 'T ) [ ] helper ;
```

```text
error: `helper` is not permitted on a quotation literal in `caller` (line 2)
```

## Recon (verified against the source at HEAD, not inferred)

1. **`call`'s rejection is a hardcoded name-list, checked against the whole stack, not the
   operand.** `poly_call_term` (`src/check/poly.rs:914-925`):
   `if matches!(name, "call" | "branch" | "if" | "times" | "tag") && stack.iter().any(|slot|
   slot.quot.is_some())`. This does not distinguish a rowless literal (`[ ]`, `[ 1 add ]`)
   from a row-typed one; it rejects `call` on *any* quotation regardless of whether the
   consumer needs row unification. `call` is the one member of this list that never does —
   it splices the literal's own body in place, the same thing the concrete path's `call`
   does (`src/check/terms.rs:299-357`: pop the literal, fetch `prov.quotations[id].body`,
   splice via `check_terms_relaxed` against the live stack, no declared effect involved at
   all when the top is a *literal* rather than an abstract parameter). `branch`/`if`/
   `times`/`tag` are genuinely row-typed and must keep the rejection — P7.S3b-follow's job,
   not this slice's.

2. **A second, earlier rejection blocks the other candidate consumer: passing a literal to
   an ordinary env word.** The operand-window guard just above (`poly.rs:966-980`) checks
   *every* candidate name (not just the combinator list) against `BUILTIN_TABLE`'s declared
   arity and rejects a `QuotLit` found in that window unconditionally, before the `env`
   dispatch below ever runs. Probe-verified: calling an ordinary (non-builtin-named) word
   with a quotation-literal operand hits this first, independent of whether the callee's
   declared parameter is `Type::InlineQuotation`/`Type::Quotation` or something else
   entirely. This is the second thing this slice must carve an exception into.

3. **Even if (2) is carved out, the `env`-dispatch loop below it (`poly.rs:997-1043`)
   cannot ground a quotation operand either.** Its per-input match is
   `PolyType::Concrete(t) if t == inp`, `PolyType::Var(v) => …`, `other => …error`. A
   `QuotLit` slot falls into `other` and errors. This loop has no arm that recognizes a
   declared `Type::Quotation`/`Type::InlineQuotation` input and grounds the literal against
   it. This is the actual grounding gap for the "call another word with a quotation
   argument" consumer, distinct from the guard in (2) that currently pre-empts it.

4. **A prerequisite gate blocks the most natural-looking consumer outright, and this slice
   does not touch it.** A **non-inline** word cannot even *declare* a `~[ ]`
   (`InlineQuotation`) parameter — probe-verified: `` word `X` declares an inline-quotation
   parameter ... but is not `inline` ``; the word must declare `inline`. So a non-inline
   generic word can never receive a comparator as a *parameter*; the only way a rowless
   quotation reaches a non-inline poly body at all is as a **literal written in the body
   itself** (`[ ... ]`), consumed there by `call` or passed to something else. `sort`'s and
   `bin_search-helper`'s own `~[ 'T 'T -- i64 ]` comparator parameters
   (`examples/experiments/arrays.sth`) stay `inline`-only after this slice; this slice does
   not make them monomorphizable. (That gate is the standing limit P7.S3b-follow's brief
   already named as untouched by either slice.)

5. **Grounding machinery for a declared quotation effect already exists and is reused
   elsewhere, but at a different call site than the one that matters here.**
   `unify_poly_input`'s `PolyType::Quotation` arm (`poly.rs:2566-2604`) unifies a declared
   quotation effect against a concrete `Type::Quotation`/`Type::InlineQuotation`
   pointwise — but its caller, `check_poly_call`, is the *concrete-word-calls-poly-callee*
   path (`poly.rs:2383+`) and explicitly rejects a quotation *argument* before ever reaching
   it (`R9p`, `poly.rs:2416-2419`, `reject_quotation_argument`). This is the wrong side of
   the boundary: `sig.inputs[i]` there are the poly *callee's own* `PolyType`s, not a
   monomorphic `env` candidate's `Type`s. The comparison this slice needs — a `QuotLit`
   slot in a poly *body* against a monomorphic candidate's ground `Type::Quotation` input —
   has no existing call site; the arm exists, but wired to a different pair of endpoints.

## The consumer

Two candidate consumers, and only one survives gate 4 above.

1. *A comparator parameter reaching a non-inline `sort`/`bin_search`.* Excluded by finding
   4: the parameter itself cannot be declared on a non-inline word, independent of this
   slice. Confirms P7.S3b-follow's existing note that this stays out of reach either way.
2. *A body-local literal, `call`ed or passed to a helper.* The only shape actually
   reachable. `[ ] call` and `[ 1 add ] call` are both rejected today for the same reason
   (finding 1) and would both compile after this slice with no row involved. A degenerate
   golden, but an honest one: this slice's exit criterion is narrower than "a comparator
   works in a non-inline `sort`" — it is "a rowless quotation literal is no longer flatly
   forbidden in a non-inline generic body."

**This narrows the slice's payoff relative to how P7.S3d was described when P7.S3b-follow
deferred it.** The text that named this slice ("splice a second, fully concrete quotation
consumer ... e.g. a comparator") assumed a comparator *parameter* was the target; finding 4
says that consumer needs the `inline`-only gate lifted too, which is out of scope for both
this slice and P7.S3b-follow. What is left is: `call` on a literal (real, findings 1–3), and
passing a literal to another *non-inline* word whose declared input is a materialized
`Type::Quotation` — which by P7.S3b's own rule ("only splice-consumed literals... nothing is
materialized") is exactly the thing that must stay rejected, not the thing to enable. So
finding 3's grounding gap may have **no legal consumer at all** once P7.S3b's
no-materialization rule is applied to it: a `Type::Quotation` parameter that isn't
`InlineQuotation` means the callee receives (and could store/return) the value, which is
materialization by another name. The only sound target left is **`call` on a literal**
(finding 1) — the spec should confirm this explicitly rather than build (2)/(3)'s carve-outs
for a consumer that turns out not to exist.

## Shape of the work

Narrower than the slice's name implied before this recon:

- Split the hardcoded list at `poly.rs:914-925`: `call` on a `QuotLit` gets its own arm —
  pop the marker, look up the literal's stored body (the `PolyQuotLit` behind the
  `PolyQuotRef`, via `scope.quotation(quot)`), and `poly_walk` that body in place against
  the current stack, the poly analogue of `check_terms_relaxed`'s splice in
  `terms.rs:299-357`. `branch`/`if`/`times`/`tag` keep the existing rejection unchanged.
- Leave the operand-window guard (finding 2) and the `env`-dispatch grounding gap
  (finding 3) alone unless the consumer analysis above is wrong and a legal
  pass-to-another-word case turns up — the working assumption going into the spec is that
  there is none, so building carve-outs for both would be machinery for a consumer that
  cannot legally exist under P7.S3b's own rule.

## Locked decisions carried forward

**Splice-consumed literals only, still.** P7.S3b's rule stands: a quotation cannot be
returned, stored, captured, or handed to a materializing consumer. `call` on a literal is
consumption, exactly like an eliminator arm; nothing here weakens that boundary.

**Type variables and rows stay rigid.** No mid-body `Subst`, no row inference — this slice
adds no row machinery at all, unlike P7.S3b-follow.

## Open questions

1. **Does `call`-splicing a literal need any join/merge logic, or is it a straight-line
   walk?** Unlike an eliminator's N arms, `call` has exactly one body and one continuation;
   there should be no analogue of `poly_eliminator_call`'s per-arm clone-and-union. Confirm
   this is genuinely simpler than S3b's machinery, not a hidden case of it.
2. **Is the "no legal consumer beyond `call`" conclusion (The consumer, above) right?**
   If a real program needs to pass a literal to another non-inline word without that word
   materializing it (e.g. the callee itself immediately `call`s it and never stores it),
   that is a real consumer P7.S3b's rule doesn't obviously forbid, and findings 2/3 would
   need their carve-outs after all. Test this before locking the spec's scope to `call`
   alone.
3. **Lowering:** does a spliced `call` inside a poly body's IR generation already fall out
   of however P7.S3b's eliminator lowering handles a spliced arm, or is this new lowering
   work? Not traced in this brief — the recon above is checker-only.

## Out of scope

- `branch`/`if`/`times`/`tag` on any quotation (row-typed or not) — P7.S3b-follow.
- A `~[ ]` parameter on a non-inline word — a standing gate this slice does not touch
  (finding 4), which is what keeps `sort`/`bin_search` from ever becoming this slice's
  consumer.
- Trait bounds (P7.S3e) and self-recursion (P7.S3g) — no interaction traced.
- Any lowering change beyond what falls out of the checker fix, pending open question 3.

## The golden

`[ ] call` and a non-trivial `[ ... ]` body (e.g. `[ 1 add ]` or a body that names/consumes
a bound local) inside a **non-inline** generic word, both compiling and running with the
correct result. Plus the negative: `branch`/`if`/`times`/`tag` on a quotation still reject
with the unchanged P7.S3b-follow message, so the split doesn't accidentally widen past
`call`.

## Ready to spec?

**Almost, but open question 2 should be answered by probing first, not assumed.** The
recon revises the slice down from "the cheap tier of a two-tier deferral" to something
smaller still: a single hardcoded-list carve-out for `call`, because the comparator-shaped
consumer P7.S3d was named for turns out blocked by an unrelated gate (finding 4), and the
"pass a literal to another word" consumer (findings 2/3) may not legally exist once P7.S3b's
no-materialization rule is applied to it. If probing confirms open question 2's answer is
"no," this is an **S**, not the **M** implied by the original deferral text — a spec should
say so plainly rather than build unused grounding machinery for findings 2/3.
