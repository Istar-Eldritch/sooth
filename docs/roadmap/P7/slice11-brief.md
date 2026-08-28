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

## Probe findings (2026-08-28, against current `main`)

**(a) Threading `Some(&generics_cell)` into `check_poly_combinator_standalone` is
mechanically trivial** — the cell is already alive in `check.rs::check`'s enclosing scope
(`generics_cell`, built once at the top of the function and passed `Some(&generics_cell)` to
every other check path) and simply wasn't passed to this one call site. Threading it through
`check_poly_combinator_standalone`'s new parameter and into its two `apply_subst` calls (the
combinator's own declared input/output slots) and its inner `check_word` call builds clean.
With only that change, a **signature-level** `Generic` type — e.g. an `Option['T]` input —
still hits a different, already-shipped rejection first: P7.S12's standing
"variable-bearing application" restriction on a combinator's *declared slot*
(`a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire`,
`tests/phase7_slice12.rs:635`), which fires during `apply_subst` on the signature *before*
`ctx.generics()` is ever consulted for the body. So the fixture this slice actually needs is
output-only construction (`map`'s shape: a concrete/quotation input, an `Option['U]`
output), not a `Option['T]`-typed input parameter — confirmed live:

```sooth
type: Result['T 'E] | Ok 'T | Err 'E ;
: wrap inline ( 'T ~[ 'T -- 'U ] -- Result['U i64] ) call Ok ;
```

fails today with exactly the predicted `poly_generic_not_yet_groundable_error`.

**(b) Threading the cell alone does not fix construction.** `check_poly_combinator_standalone`
builds a fully *concrete* `WordDef` (`poly: None`) and checks its body with `check_word` →
`check_terms_word`, the ordinary **concrete** term walk. `poly_construct_generic` — the
mechanism that lets a real poly body's own term walk (`poly_call_term`, only reached from
`check_terms_relaxed`) construct a `PolyType::Generic` output — is never called from the
concrete path at all. The concrete walk's only way to resolve a variant-constructor name
like `Ok` is an ordinary `env.get(name)` lookup, which only has an entry if *some other
monomorph of that generic enum was already minted and registered elsewhere in the same
program* (verified: adding an unrelated `: mki ( i64 -- Result[i64 i64] ) Ok ;` to the same
file makes the "unknown word `Ok`" error disappear, replaced by an unrelated call-site
output-inference error on `wrap`'s own `'U`). A library file defining `map`/`and_then` with
nothing else in it instantiating `Option[i64]` first has no such accidental monomorph, so
threading the cell is not sufficient on its own — the gap is real construction machinery for
the standalone-checked body, not just a `None` guard to lift.

This sharpens the brief's own open question: it is not "loosen the standalone gate" (there
is no gate on the body path to loosen — the concrete checker simply has no construction
mechanism for a `Generic` output at all) but **which of two real designs to build**:

- **Route the standalone body through the poly-body term walk** (`check_terms_relaxed` /
  `poly_call_term`) instead of the concrete one, so `poly_construct_generic`'s existing
  by-name search over `structs`/`enums`/`ctx.generics()` fires for real, minting a stand-in
  monomorph (`Option[i64]`, keyed the same way an ordinary poly word's own construction
  mints one today) the first time a combinator body constructs one — no dependency on any
  other word in the program having minted it first. This is the larger change: the concrete
  stand-in `WordDef`/`check_word` call would need replacing or wrapping with the poly path,
  and every other already-working concrete-checker behaviour the standalone pass currently
  gets for free (quotation `call`/`times`, R8/R9, R16) would need to keep working under the
  poly walk instead.
- **Pre-mint a stand-in monomorph before the concrete check runs**, registering its
  constructors into a scratch `env` clone the same way `apply_subst`'s existing
  `Generic`-grounding arm mints one for a *signature slot* — i.e. do for the body's
  constructor names what the signature-slot fix above already does for declared
  inputs/outputs, without switching the body walk off the concrete checker. Smaller change,
  but needs its own answer to "keyed on what module and what dedup identity" for a
  monomorph that exists only because the *standalone* stand-in check needed it, never a
  real call site — the same identity question the brief's second bullet already flagged.

## Ready to spec: yes

The two probes above resolve the brief's structural uncertainty (is this a small threading
fix or a distinct mechanism → confirmed the latter) and supply a real golden
(`wrap`/`Result['U i64]`, or the equivalent `lib/core/option.sth::map`). The spec still has
a design choice to make between the two routes above; that choice belongs in the spec, not
this brief.
