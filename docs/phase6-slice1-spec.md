# Phase 6 Slice 1: quotation effect annotations (spec)

A quotation literal (`[ ... ]` / `~[ ... ]`) may carry an optional leading
parenthesized effect inside its own brackets: `[ ( ..a T -- ..b ) term* ]`. The
annotation is checked against the literal's own body, and, where a consuming
context also declares a quotation effect, reconciled against that context. This
slice mints no eliminator, variant type, or per-variant accessor: it is a
parser + checker feature only, additive to the existing grammar.

Recon in [`docs/phase6-slice1-brief.md`](./phase6-slice1-brief.md) holds as
written; source line references below were re-read against `main` at `9f1884e`
(`cargo test` green).

## Grammar (D1)

The annotation is an **optional** parenthesized effect immediately after the
opening bracket, before the body terms:

```text
quotation-literal := ( "[" | "~[" ) annotation? term* "]"
annotation        := "(" effect-list "--" effect-list ")"
```

- A literal with no leading `(` parses **exactly** as today (recon 1): the
  `parse_term` arms at `src/parser.rs:3139` / `:3155` are unchanged for the
  no-annotation path. This slice is purely additive; every existing quotation
  literal keeps its current parse.
- The leading `(` is the sole disambiguator. Inside `parse_term`'s `LBracket` /
  `TildeLBracket` arms, after consuming the opening bracket, peek for
  `Token::LParen`; if present, read the annotation, then read the body with the
  existing `parse_terms` reader. The body reader itself is untouched.
- The effect-list reader is a **new, variable-aware** reader; it is *not*
  `parse_quot_type_list` reused verbatim. That existing reader
  (`src/parser.rs:2299`) stops on `Token::RBracket` (`]`), not `)`, and calls
  `parse_type_expr`, which resolves only **concrete** types and rejects the
  `'T` / `..a` variable forms outright. This slice follows the general
  *shape/pattern* of `parse_quot_type_list` (read a list, then `--`, then a
  second list) but the new reader stops on `)`, admits variable spellings, and
  mints each fresh `'T` / `..a` into the literal's own per-literal id space,
  mirroring how `PolySig` / `parse_poly_effect` (`src/parser.rs:1614`) mint ids
  but in a **disjoint per-literal space**, never the enclosing word's `PolySig`.
- A body term can never itself legally begin with `(` in the current grammar:
  `parse_term` has no `Token::LParen` arm, so a `(` at a body-term position
  falls through to its catch-all `unexpected token` located parse error
  (the `other =>` arm at `src/parser.rs:3171-3172`, and see recon 2). The leading-`(` disambiguator is
  therefore unambiguous: a `(` immediately after the opening bracket can only
  be the annotation, and there is no valid body term it could be confused with.

### Variables in the annotation (D2, and see OQ3)

The effect may name type variables (`'T`) and rows (`..a`), scoped to a **new
per-literal variable space**, never the enclosing word's `PolySig` (recon 4).
Two sibling literals in one word body get independent ids; nothing unifies
across them except through an existing combinator parameter that already ties
them (e.g. `times`'s `~[ 'a -- 'a bool ]`).

`parse_quot_type_list` today calls `parse_type_expr`, which resolves only
**concrete** types (no variable forms; those live in the poly-signature path via
`PolyType`). Supporting `'T` / `..a` here therefore requires a variable-aware
list reader that records each fresh spelling into the literal's own local name
tables. See R2 and R6 for how far this slice takes that.

## AST (D4)

`TermKind::Quotation` (`src/ast.rs:1559`) gains one optional field carrying the
annotation; **no new `Type` variant** is introduced (D4 exit criterion).

```rust
Quotation(Vec<Term>, bool, Option<QuotAnnot>)
```

`QuotAnnot` is a self-contained effect (it has no enclosing `PolySig` to borrow
a row-id space from, recon 4):

```rust
pub struct QuotAnnot {
    pub inputs: Vec<PolyType>,
    pub outputs: Vec<PolyType>,
    pub row_in: Option<u32>,
    pub row_out: Option<u32>,
    pub ty_var_names: Vec<String>,
    pub row_var_names: Vec<String>,
    pub span: Span,
}
```

- A fully-concrete annotation leaves `row_in` / `row_out` `None` and both name
  tables empty; its `inputs` / `outputs` are all `PolyType::Concrete`.
- The variable spaces are the annotation's own (`Var(0)` in one literal's annot
  is unrelated to `Var(0)` in another), mirroring `PolySig`'s per-signature id
  spaces (`src/ast.rs:989-1001`) but minted per literal.
- `span` is the annotation's opening `(`, used for the standalone-mismatch
  diagnostic (R3).

## Checking

### R1 — an annotated literal is always checked against its own annotation

Recon 3: today no code path computes a literal's standalone effect; every
effect check funnels through `check_literal_against_declared_effect`
(`src/check.rs:1408`) comparing against a **caller-supplied** `&QuotEffect`.
This slice adds the missing standalone path.

For an annotated literal, seed a fresh sub-stack from the annotation's declared
input row and run the literal's body against it, **reusing the directional
D3-style check** that `check_literal_against_declared_effect` already performs
(seed input row, run body, require exit row to equal the declared output row;
`src/check.rs:1408` doc block). The standalone path uses the annotation as that
declared effect instead of a parameter's. This is the same
`quot_bodies`-tracked compile-time-literal check `5749a14` established for
row-carried quotations (recon 5): no materialization boundary is exercised.

### R2 — variable binding in a standalone annotation

> **Amended after implementation review (round 1).** The paragraph below is the
> *original* design; it does not describe what shipped. `AnnotEffect` (the
> resolved form every downstream check reads) has **no row field at all** --
> `resolve_annotation` reads `row_in`/`row_out` only long enough to compare
> them to each other, then discards them. A row that survived resolution
> unrejected would therefore be checked against nothing, ever: not the
> standalone body (only `annot.inputs`/`annot.outputs`, the *named* slots, seed
> the sub-stack) and not a consuming parameter's own row
> (`reconcile_annotation_with_parameter` compares only `inputs`/`outputs`
> vectors). That is a strict-equality hole (R5) large enough to accept an
> annotation naming a row a parameter doesn't have at all, and, separately,
> it makes a real accumulator-threading body (the idiom `fold`/`filter` are
> built on) unannotatable with any message that isn't a confusing downstream
> underflow. **Implemented decision:** any annotation naming a row --
> passthrough (`..a -- ..a`) included, not only shape-changing -- is a located
> error, unconditionally, in this slice. The *shape-changing* case keeps its
> own "no fixed point" wording (below); a passthrough row gets a new, distinct
> wording ("row ... is not supported") for the different reason (there is a
> fixed point, but this slice tracks nothing to check it against). Giving a row
> annotation real standalone meaning -- and a real R4 bridge to a consuming
> parameter's row -- is deferred to whichever later slice needs it; it needs an
> `AnnotEffect` row field this slice never built, not just relaxing the check
> below.

A row variable (`..a`) in a standalone annotation denotes the literal's own
input/output row: it seeds and is matched against the fresh sub-stack's row, the
same way a plain word's declared row unifies against its own body. A concrete or
row-only annotation is fully checkable standalone.

A **type variable** (`'T`) in a standalone annotation has no binder: nothing in
a freestanding literal ever supplies its instantiation. Such a literal, checked
with no consuming parameter, is a **located error** at the annotation span:

```text
error: effect variable `'T` in a quotation annotation is unbound (line L)
  a type variable in an annotation only binds where the literal fills a declared
  quotation parameter that supplies it
```

(The message carries only `(line L)`, matching every existing `check.rs`
diagnostic, which locate to a line, never a column.)

This is the concrete resolution of OQ3 for the type-variable case: a
freestanding `'T` is meaningless (nothing consumes it), so it is rejected rather
than silently accepted or given an invented meaning.

> **Amended after implementation review (round 1).** Unlike the row case, the
> *rejection* here shipped exactly as specified above -- but the paragraph's
> premise that a type variable later "binds where the literal fills a declared
> quotation parameter" (feeding into R4) does not. `resolve_annot_slots` (the
> function behind this rejection) runs once, at the literal's interning site,
> **before any consuming parameter is known**, and rejects *every*
> `PolyType::Var` unconditionally -- there is no code path where a `'T` survives
> resolution to be looked up against a poly parameter's ground later. R4 (below)
> is amended to match: it never sees a `Var`, because none can reach it.
> **Implemented decision:** a standalone type variable is rejected
> unconditionally in this slice, full stop, independent of whether the literal
> goes on to fill a parameter. Binding it against a poly parameter's ground
> (the deferred-resolution design the original paragraph describes) needs the
> annotation's own `Var` ids to survive past intern time to the argument-
> matching site, plus a sweep that errors on any that are never reconciled --
> machinery this slice does not build. Deferred to whichever later slice needs
> it.
>
> ```text
> : on inline ( 'T ~[ 'T -- 'T ] -- 'T ) | f | f call ;
> : main ( -- ) true ~[ ( 'T -- 'T ) dup drop ] on drop ;
> → error: effect variable `'T` ... is unbound (rejected at the literal, not reconciled at `on`)
> ```

A **shape-changing row** (`..a -- ..b`, row inputs and outputs differing) is a
**located error** at the annotation span. A freestanding shape-changing effect
has no fixed point to check against (nothing supplies the difference between
`..a` and `..b`):

```text
error: shape-changing row `..a -- ..b` in a quotation annotation is unbound (line L)
  a standalone shape-changing row has no fixed point to check against; a
  shape-changing row annotation that fills a quotation parameter is out of
  scope for this slice
```

Slice 1 specifies **only** this standalone rejection. A shape-changing row
annotation that fills a (poly) parameter is explicitly out of scope (see *Out of
scope*): `check_literal_against_declared_effect`'s own doc block
(`src/check.rs:1400-1407`) notes that a quotation whose declared effect is
itself shape-changing has no standalone fixed point, and its exit row is
discovered by forward-checking and compared only against a *sibling* literal in
`check_poly_combinator_args`, never grounded the way a type-variable position
is. R4's positional bridge grounds **type-variable positions only, never row
positions**, so nothing in this slice grounds a shape-changing row annotation
that fills a parameter; defer it to whichever later slice needs it.

Only a **passthrough** row (`..a -- ..a`) or a fully concrete effect is
self-checking standalone: a passthrough constrains only the body's net stack
effect (the same region on both sides), not any element type, and a concrete
effect has no variable to bind at all.

### R3 — body/annotation disagreement is a located standalone error

When the literal's actual body effect disagrees with its own annotation, emit a
located error at the annotation span, **independent of whether the literal fills
any parameter**. Reuse R11's existing mismatch diagnostic shape
(`literal_effect_mismatch_error`, `src/check.rs:1661`) rather than inventing a
second wording for an effect disagreement.

### R4 — reconciliation against a declared parameter (D3)

R4 is scoped down to **only the poly-parameter case** (see the amendment below
for why this is narrower than "type-variable case"). For a concrete/mono
declared parameter, R4 does no independent work: R3 (body-vs-annotation) plus
the pre-existing R11 body-vs-parameter check
(`check_literal_against_declared_effect` -> `literal_effect_mismatch_error`,
`src/check.rs:1661`) already force `body == annotation == parameter`
transitively under R5's strict equality. A concrete-parameter R4 test would
therefore be a **placebo** (R3 and R11 both already fire), so R4 mints no
separate concrete-parameter check and no concrete-parameter test.

Where the literal fills a **poly** parameter, R3's directional check does not
constrain the annotation against a polymorphic/identity body: an
identity-shaped body (`'a -- 'a`) absorbs whatever concrete type the annotation
claims without contradiction, so R3 never pins it, and closing that gap is
exactly R4's exclusive job (which is why the isolating test below requires a
polymorphic body, not because it duplicates R3). By the time
`check_literal_against_declared_effect` runs, the parameter's `eff:
&QuotEffect` is **already grounded**: `PolyCtx`'s substitution has replaced each
declared type variable with its concrete ground in `eff.inputs` / `eff.outputs`.

> **Amended after implementation review (round 1).** The design above (a
> positional walk that binds an annotation `Var` to the parameter's grounded
> type) is **unreachable** as built, and is not what shipped. R2's amendment
> means `resolve_annotation` has already rejected every `PolyType::Var` before
> `AnnotEffect` exists -- `annot.inputs`/`annot.outputs` are `Vec<Type>`
> (fully-resolved, no `Var` variant can survive to reach here) by the time R4
> runs, for *every* annotated literal, poly-parameter-filling or not.
> **Implemented decision:** R4 is plain vector equality,
> `annot.inputs == eff.inputs && annot.outputs == eff.outputs`
> (`reconcile_annotation_with_parameter`, `src/check.rs`) -- there is no
> "annotation position is a `Var`" branch to write, because a `Var` can never
> arrive. What still makes this real work R3/R11 cannot do is exactly the
> concrete-vs-grounded comparison: the annotation's own concrete claim
> (`i64`) against the parameter's *substituted* ground (`bool`), which an
> identity body absorbs on both sides without R3 or R11 ever seeing the
> conflict. A `'T`-spelled annotation at a poly-parameter position is out of
> scope for this slice (R2's amendment): it is rejected at the literal before
> any parameter context exists, never reconciled here.
>
> **Amended after implementation review (round 2).** R4 as first built was
> skipped entirely for a *shape-changing* declared parameter
> (`~[ ..i -- ..o ]`, `..i != ..o`), on the reasoning that such a parameter has
> no fixed point. That left a hole with the same shape R4 exists to close: a
> fully concrete annotation flatly contradicting the parameter's fixed slots
> (`~[ ( bool -- bool ) dup drop ]` against a declared `~[ ..i i64 -- ..o i64 ]`)
> passed every check, since R3 seeds from the annotation, R11 holds only the
> declared *suffix*, and an identity body absorbs both. **Implemented
> decision:** R4 also runs for a shape-changing parameter, comparing the
> annotation and the declaration where both are determined -- their overlapping
> tails. Full vector equality would be wrong there: the parameter names only
> the slots above the row, so a literal may legitimately reach past them into
> the row (`( i64 -- )` at a declared `~[ ..i -- ..o ]`) or leave more behind
> than the declaration names (`( -- i64 )`), and both of those check today.

A disagreement is R11's error (`literal_effect_mismatch_error`,
`src/check.rs:1661`), naming both the grounded parameter effect and the
annotation's effect.

### R5 — strict equality, no subtyping (resolves OQ1)

Recon found no subtyping concept anywhere in `Type`. The annotation-vs-parameter
check and the annotation-vs-body check are both **strict structural equality**
of the (grounded) effect rows. There is no narrowing, no "annotation confirms
but is otherwise ignored," and no compatible-but-not-identical acceptance. A
disagreement is always an error (R3 for the body, R4 for the parameter); an
identical annotation is a no-op confirmation. If a future slice introduces
effect subtyping, that relaxation is its own decision, not carried in here.

### R6 — full form only, no elision (resolves OQ2)

Slice 1 parses only the full four-part effect `( inputs -- outputs )` (either
side possibly empty of *named types* but the `--` always present). No elided
form is accepted:

- `( )` with no `--` is a located parse error.
- A missing `--` inside the annotation parens is a located parse error from the
  **new** variable-aware effect-list reader (D1). It is not the existing
  `parse_quot_type_list` path: that reader stops on `]`, whereas the new reader
  stops on `)` and raises its own "expected `--`" located error when the
  closing `)` arrives with no separator seen.
- Arm-position elision (`( Circle )`, partial effects) is **out of scope** and
  belongs to Slice 3, which needs it for variant binding, a different purpose.
  Conflating the two here would ship a general partial-effect mechanism nobody
  asked for.

## OQ resolutions (summary)

- **OQ1** → R5: strict equality, no subtyping/narrowing. Disagreement is always
  an error.
- **OQ2** → R6: full unelided form only; all elision deferred to Slice 3.
- **OQ3** → R1/R2 (amended after implementation review, round 1): a fully
  concrete effect is checked in a per-literal space against the literal's own
  body (standalone-meaningful). A standalone type variable, a standalone
  shape-changing row (`..a -- ..b`), **and** a standalone passthrough row
  (`..a -- ..a`) are each a located error, unconditionally, in this slice --
  `AnnotEffect` was never given a row field, so a row admitted here would be
  checked against nothing ever again, and `resolve_annot_slots` rejects every
  type variable before any consuming parameter is known, so it never reaches a
  poly parameter's ground either. Neither case "binds when a poly parameter
  consumes the literal": R4 only ever compares fully-resolved `Vec<Type>`
  vectors (see R2/R4's amendments above). The exit witness uses a concrete
  annotation, needing no variable binder.

## Out of scope

- Any enum/eliminator-shaped construct (`Type::Variant`, per-variant accessors,
  the eliminator word, arm-position elision `( Circle )`): all later Phase 6
  slices.
- Any `src/ir/` or backend change: recon 5-6 found no lowering gap this slice's
  exit case exercises (row-carried literal quotations already lower and run,
  `tests/phase4_slice10b.rs`). An annotated literal used as an ordinary
  `[ ... ]` / `~[ ... ]` argument reuses that path unchanged.
- **Any row annotation with real meaning** — passthrough (`..a -- ..a`) or
  shape-changing (`..a -- ..b`), standalone or filling a quotation parameter
  (amended after implementation review, round 1; supersedes the narrower
  "shape-changing only" scoping this bullet originally had). Slice 1 rejects
  every row-bearing annotation outright, unconditionally: `AnnotEffect` has no
  row field, so nothing this slice built could check one against anything.
  Giving a row annotation standalone meaning, and a real R4 bridge to a
  consuming parameter's row, both need machinery (an `AnnotEffect` row field,
  plus whatever comparison it feeds) this slice does not build, deferred to a
  later slice.
- **A `'T`-spelled annotation reconciled against a poly parameter's ground**
  (amended after implementation review, round 1). `resolve_annot_slots` rejects
  every type variable unconditionally, before any consuming parameter is
  known, so R4 never sees one to bind; the positional Var-binding bridge R4
  originally described is unbuilt, and needs the annotation's `Var` ids to
  survive past intern time. Deferred to a later slice.
- **The annotation is not checked against the stack at a direct `call`**
  (observed in implementation review, round 2). R3 holds the annotation against
  the literal's own body and R4 against a *declared* quotation parameter, but
  `[ ( bool -- bool ) dup drop ] call` on an `i64` checks and runs: the
  annotation is never held against the actual stack the literal is invoked on.
  This follows R4's scoping ("where a consuming context *declares* a quotation
  effect") rather than contradicting it, but `call` is the most common
  consumption path, so what slice 1 ships is narrower than "annotations are
  checked". Whichever later slice wants it owns the new check.
- Subtyping / effect-narrowing (R5): deferred unless a concrete need appears.
- `while`'s materialized-quotation rejection (recon 5): unrelated, unchanged.

## Files touched

- `src/ast.rs`: the new `Option<QuotAnnot>` field on `TermKind::Quotation` and
  the `QuotAnnot` struct (no new `Type` variant, D4).
- `src/parser.rs`: the leading-`(` annotation reader inside the `LBracket` /
  `TildeLBracket` arms of `parse_term` (`:3139` / `:3155`), plus the **new**
  variable-aware effect-list reader (D1/R2, following the shape of
  `parse_quot_type_list` at `:2299` but not reusing it) and the
  elision-rejection errors (R6).
- **Mechanical arity padding across the tree (Phase 1 scope).** Changing
  `TermKind::Quotation(Vec<Term>, bool)` to
  `Quotation(Vec<Term>, bool, Option<QuotAnnot>)` breaks every existing
  pattern-match and construction site, so each needs a mechanical `_` (pattern)
  or `None` (construction) added. Phase 1 must compile green standalone, so
  these edits belong to Phase 1, not left implicit. Re-grepped `TermKind::Quotation`
  sites at `9f1884e`: `src/resolve.rs` (`:474`, `:766`), `src/repl.rs` (`:305`,
  `:306` construction, `:2175`), `src/ast.rs` (`:1630`, `:1632` construction),
  `src/check/terms.rs` (`:789`), `src/check/engine.rs` (`:579`, `:666`, `:711`,
  `:740`, `:1976`), `src/check/poly.rs` (`:444`), `src/check/captures.rs`
  (`:30`), `src/check/drop_graph.rs` (`:294`, `:725`), `src/ir/func_builder/mod.rs`
  (`:69`), `src/ir/func_builder/calls.rs` (`:55`, `:747`), and the internal
  `parser.rs` construction/pattern sites (`:3145`, `:3161` construction; `:3330`,
  `:3338`, `:3436`, `:3445`, `:3449`, `:3470`, `:3479`, `:3483` patterns).
- `src/check.rs`: the new standalone body-vs-annotation check (R1/R3) and the
  unbound-variable / shape-changing-row errors (R2), plus the poly-parameter
  reconciliation branch in `check_literal_against_declared_effect` (`:1408`)
  for R4/R5.

## Exit

An annotated quotation literal whose body effect disagrees with its own
annotation is a located error, independent of whether it fills any parameter
(R3). An annotated literal that also fills a declared quotation parameter and
disagrees with that parameter's effect is a located error naming both effects
(R4). An unannotated literal anywhere in the tree is unaffected: the full
existing suite stays green with no changes to any file outside the annotation
path (D4).

The D4 "no new `Type` variant" constraint is enforced by **code review of
`src/ast.rs`**, not by any runtime test: no test can assert the absence of a
variant, so a reviewer must confirm the AST change is the single
`Option<QuotAnnot>` field plus the `QuotAnnot` struct and nothing on `Type`.

The R2 unbound-type-variable and unbound-shape-changing-row diagnostics are
covered by **unit tests only** (below), deliberately with no golden. They are a
checker-internal rejection with no separate build artifact to diff, and the
two headline exit goldens (body mismatch, parameter mismatch) already exercise
the located-error diagnostic machinery end to end; a third and fourth golden
for the unbound cases would duplicate that plumbing without adding coverage the
unit tests lack.

## Tests (goldens + unit)

Parser (`src/parser.rs` `#[cfg(test)]`):

- `parse_quotation_annotation_full_form_ok` — `[ ( i64 -- bool ) dup 10 < ]`
  parses to a `Quotation` with a concrete `QuotAnnot`.
- `parse_quotation_annotation_inline_flavour_ok` — same for `~[ ( ... ) ... ]`,
  flavour flag still `true`.
- `parse_quotation_no_annotation_unchanged` — `[ dup 10 < ]` parses with
  `annotation: None` (additive-parse guard).
- `parse_quotation_annotation_missing_arrow_is_error` — `[ ( i64 bool ) ... ]`
  is a located parse error (R6).
- `parse_quotation_annotation_elided_is_error` — `[ ( ) ... ]` is a located
  parse error (R6).
- `parse_quotation_annotation_row_ok` — `[ ( ..a i64 -- ..a ) ... ]` records a
  per-literal `row_in` / `row_out`. Parsing still records these; it is the
  checker (R2, amended) that rejects every row-bearing annotation at intern
  time, so this parse-level test is unaffected by that amendment.

Checker (`src/check.rs` `#[cfg(test)]`):

- `check_annotation_matches_body_ok` — `[ ( i64 -- bool ) dup 10 < ]` checks.
- `check_annotation_disagrees_with_body_is_error` — `[ ( i64 -- i64 ) dup 10 < ]`
  is a located mismatch error at the annotation span (R3), with **no** consuming
  parameter present.
- `check_annotation_disagrees_with_poly_parameter_is_error` — a poly quotation
  parameter word (trivially definable, patterned on `lib/combinators.sth`'s
  poly quotation parameters), e.g.

  ```text
  : on inline ( 'T ~[ 'T -- 'T ] -- 'T ) | f | f call ;
  ```

  (`inline` is required: `check_inline_quotation_requires_inline`,
  `src/check/word_entry.rs:118`, rejects any word declaring a `~[ ... ]` poly
  quotation parameter that is not itself `inline` — every real `~[ ]`-taking
  word in `lib/combinators.sth`/`lib/core.sth` is `inline` for this reason.
  Omitting it here would fail every test below on the wrong diagnostic before
  R4 ever runs.) Called as `true ~[ ( i64 -- i64 ) dup drop ] on`. The value
  `true` grounds
  `'T` to `bool`, so the parameter's already-substituted `eff` is
  `~[ bool -- bool ]`, while the annotation names `( i64 -- i64 )` at that
  position. The body `dup drop` is net identity (`'a -- 'a`), so it is
  **polymorphic** at the position in question: R3 (body-vs-annotation) cannot
  fire, because an identity body absorbs the annotation's `i64` claim without
  contradiction; R11 (body-vs-parameter) cannot fire, because it absorbs the
  grounded `bool` just as readily. Only R4's plain vector-equality comparison
  (annot vs the already-grounded `eff`; see R4's amendment) sees the conflict
  (annotation's concrete `i64` vs the parameter's grounded `bool`).
  Emits R11's mismatch naming both effects (R4/R5). This replaces the earlier
  mono-parameter example (a placebo: R3 + R11 already fire) and the earlier
  `i64`-body/`bool`-annotation example (also a placebo: with a concrete `i64`
  body, R3 fires directly on `i64` vs `bool`, never reaching R4). The test must
  assert the **exact** R11 mismatch message text (naming both effects), not
  merely `is_err()`: an `is_err()`-only assertion cannot distinguish R4 firing
  from an unrelated error (e.g. the inline-requires-inline check above, if `on`
  were ever misdeclared) firing for the wrong reason.
- `check_annotation_agrees_with_poly_parameter_ok` — the same `on` word called
  as `true ~[ ( bool -- bool ) dup drop ] on`: the annotation's concrete claim
  matches the parameter's grounded `bool`, so R4 is an identity no-op and the
  literal checks (R5).
- `check_annotation_disagrees_with_shape_changing_parameter_is_error` (round 2,
  see R4's second amendment) — `~[ ( bool -- bool ) dup drop ]` filling a
  declared `~[ ..i i64 -- ..o i64 ]`, asserted on the exact R4 message. Nothing
  else rejects it: R3 seeds from the annotation, R11 checks only the declared
  suffix, and the identity body absorbs both claims.
- `check_annotation_extending_shape_changing_parameter_row_ok` (round 2) — the
  counterweight: `( -- i64 )` and `( i64 -- )` against a declared
  `~[ ..i -- ..o ]` both check, so the fix cannot be full vector equality.
- `check_standalone_type_variable_annotation_is_unbound_error` — a freestanding
  `[ ( 'T -- 'T ) ]` bound to a local (no consuming parameter) is the unbound
  error (R2). The test must assert the **exact** diagnostic message text (the
  `effect variable ... is unbound` wording), not merely `is_err()`: an ordinary
  row/arity mismatch from the standard body-check machinery could otherwise
  satisfy an `is_err()`-only assertion vacuously.
- `check_standalone_shape_changing_row_is_unbound_error` — a freestanding
  `[ ( ..a -- ..b ) ... ]` bound to a local (no consuming parameter) is the
  shape-changing-row unbound error (R2). Like the previous test, it must
  assert the **exact** `shape-changing row ... is unbound` message text, not
  just `is_err()`, so a plausible alternative error path (an ordinary
  row/arity mismatch) cannot satisfy it vacuously.
- `check_standalone_passthrough_row_annotation_is_unsupported_error` (amended
  after implementation review, round 1) — a freestanding passthrough
  `[ ( ..a -- ..a ) ... ]` is **also** a located error (`row ... is not
  supported`), not the self-checking success the original R2 described:
  `AnnotEffect` has no row field to check it against. Asserted on the exact
  message for the same reason as the previous test.
- `check_standalone_shape_changing_row_with_unnamed_side_has_no_trailing_space`
  (minor, round 1) — `[ ( ..a -- ) ... ]` renders `` shape-changing row
  `..a --` `` with no stray trailing space where the unnamed side would
  otherwise leave one.

Golden (source in → diagnostic / build out), the phase exit criteria:

- `phase6_slice1.rs::annotated_literal_body_mismatch_diagnostic` — the standalone
  disagreement error (Exit case 1).
- `phase6_slice1.rs::annotated_literal_parameter_mismatch_diagnostic` — the
  parameter disagreement error (Exit case 2). Uses the discriminating
  poly-parameter construction (so R4, not R3, is what fires): a source defining
  a poly quotation parameter word and calling it with a mismatched annotation
  over an identity body, e.g.

  ```text
  : on inline ( 'T ~[ 'T -- 'T ] -- 'T ) | f | f call ;
  : main ( -- ) true ~[ ( i64 -- i64 ) dup drop ] on drop ;
  ```

  `'T` grounds to `bool`, the annotation claims `i64` at that position, and the
  identity body `dup drop` keeps R3/R11 from firing, so only R4's plain
  vector-equality comparison catches the `i64`-vs-`bool` conflict; the golden
  asserts R11's mismatch diagnostic naming both effects.
- `phase6_slice1.rs::annotated_literal_agreeing_builds` — a program with a
  correctly annotated literal (both standalone and parameter-filling) builds and
  runs (Exit: additive, agreeing case is accepted).
- Regression: the existing `phase4_slice10b.rs` goldens stay green unchanged
  (unannotated literals unaffected).

**Diagnostic wording note for Phase 3 (round-1 review, minor).** R3
(`annotation_body_mismatch_error`) and R4
(`annotation_parameter_mismatch_error`) each mint their own wording rather
than reusing `literal_effect_mismatch_error`'s text verbatim, as an earlier
draft of this spec directed. The new wordings are more accurate (R4 in
particular is a declaration-vs-declaration conflict, not a body complaint), so
this is kept, not reverted -- but the goldens above must assert against the
**actual** `check.rs` message text (see the two unit tests
`check_annotation_disagrees_with_body_is_error` /
`check_annotation_disagrees_with_poly_parameter_is_error` for the exact
strings), not against R11's wording.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "AST and parser grammar: add the Option<QuotAnnot> field on TermKind::Quotation and the QuotAnnot struct (D4, no new Type variant); the arity change forces mechanical _/None padding at every existing TermKind::Quotation pattern/construction site across src/resolve.rs, src/repl.rs, src/ast.rs, src/check/{terms,engine,poly,captures,drop_graph}.rs, src/ir/func_builder/{mod,calls}.rs, and internal parser.rs (Phase 1 must compile green standalone); plus the leading-( annotation reader inside parse_term's LBracket / TildeLBracket arms, a NEW variable-aware effect-list reader following the shape of parse_quot_type_list but not reusing it (stops on ), admits 'T/..a, mints per-literal ids like parse_poly_effect but in a disjoint space, R2), and the elision/missing-arrow parse errors (R6); parser unit tests including the additive no-annotation guard",
      "effort": "M-L",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Checker rules: the standalone body-vs-annotation check (R1/R3) via the D3-style directional check, the unbound type-variable and unbound shape-changing-row errors (R2), and the scoped-down poly-parameter reconciliation branch in check_literal_against_declared_effect (R4) that walks the already-grounded parameter effect against the annotation's own per-literal Var ids positionally, under strict structural equality (R5); no concrete-parameter R4 check (placebo per R4); checker unit tests",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Goldens and regression: the phase6_slice1.rs exit goldens for the standalone and parameter mismatch diagnostics and the agreeing-builds case, plus confirming the phase4_slice10b.rs unannotated-literal goldens stay green unchanged",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
