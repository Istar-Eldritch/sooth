# P7.S11 brief — Generic construction inside an inline combinator's standalone check

## Problem, confirmed live against current `main`

`lib/option.sth` declares:

```sooth
type: Option 'T | None | Some 'T ;

: map inline ( Option['T] ~[ 'T -- 'U ] -- Option['U] )
  ...
```

`map` is `inline`: it takes a quotation parameter, so it is checked once, standalone,
against `i64` stand-ins by `check_poly_combinator_standalone` (`src/check/poly.rs:381`),
not per call site the way a plain (non-quotation-taking) generic word is. That function
builds its `Ctx` with `generics: None`:

```rust
// P7 slice 3a: construction (R3) is scoped to an ordinary poly word's
// own body, not a combinator's standalone stand-in check -- `None` here,
// never threaded in from a caller, keeps that scope decision in one
// place rather than relying on every caller to also decline it.
let ctx = word_ctx(..., None);
```

So `map`'s own body, constructing `Some result` to build the `Option['U]` output, hits:

```
error: `Some` in `map` (line N) names the generic type `Option['U]`, which cannot yet be
instantiated at a variable-bearing application
  grounding a generic over its own type variable is not yet implemented
```

(`poly_generic_not_yet_groundable_error`, `src/check/poly.rs:7408`, reached via the
`ctx.generics()` `None` arm at `:7353`/`:7548`).

This is a real gap, not a mis-scope: P7.S3a's construction/grounding machinery
(`poly_construct_generic`, `unify_poly_input`'s `Generic` arm, `apply_subst`'s `Generic`
arm) was built and wired for the ordinary poly-word path (`check::check`'s per-word `Ctx`,
which does carry `Some(&generics)`), and deliberately declined for the combinator-standalone
path in the same slice, recorded as a scope decision rather than a defect. Nothing has
revisited it since. It blocks every `Option`/`Result`-shaped `inline` combinator
(`map`, `and_then`, `unwrap_or`, ...) that needs to *construct* the generic type from inside
its own standalone-checked body — which is most of `Option`'s and `Result`'s useful
vocabulary.

## Why standalone checking exists (context for the fix)

`check_poly_combinator_standalone`'s own doc comment: a combinator's body is checked once
against `i64` stand-ins (Copy/Ord/numeric) rather than per call site, because instantiating
every type variable at the same concrete type cannot mask a real error — the combinators
never combine two distinct element/accumulator variables directly. That reasoning is about
*monomorphic* stand-in types; it says nothing about whether a *generic type application*
inside that body can ground. The two are independent design questions this brief keeps
separate: the "check once at `i64`" strategy does not have to change for a "generic
construction is groundable here too" fix to land.

## What is not yet decided (for the spec to resolve)

- **Whether standalone checking can construct at all**, given there is no real call site to
  substitute — `i64` stands in for `'T`, but `'U` in `map`'s effect has no operand to unify
  against inside the standalone pass (unlike the ordinary poly-word path, where the caller's
  actual output slot supplies it via `poly_construction_fallback`). This needs a design
  decision, not just threading `Some(&generics)` through: does the standalone pass mint a
  stand-in monomorph (`Option[i64]`) as it does for scalars, and if so, keyed on what module
  and what dedup identity?
- Whether the per-call-site *re-check* that already exists for a spliced combinator body
  (the same re-validation `poly_call_term`'s construction arm runs against each nesting
  level's live row, per P7.S3a/P7.S3b's splice model) is sufficient cover once the
  standalone pass stops rejecting outright — i.e. whether this is "loosen the standalone gate
  and let the real per-splice check catch mistakes" or "the standalone pass must itself
  ground correctly with no help from a later pass."
- Scope: this brief only covers *construction* of a `PolyType::Generic` output inside a
  combinator's own body (the `map`/`Some` case). Whether the same `None` also blocks other
  `Generic`-typed operations reachable only from a combinator body (`dup`/`over` rejection,
  `unify_poly_input`'s consuming side) needs its own probe before scoping in or out.

## Ready to spec: no — needs a design probe first

Before a spec is written, probe (a) whether `Some(&generics)` can simply be threaded into
`check_poly_combinator_standalone` and rebased the same way the ordinary path does at the
top of its own check, or whether the "no real call site" gap above forces a distinct
mechanism, and (b) a minimal repro building today's `lib/option.sth::map` end to end, to use
as this slice's golden.
