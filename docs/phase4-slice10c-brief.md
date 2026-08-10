# Phase 4 Slice 10c: `if`/`cond` as ordinary words (brief)

Originally split out of slice 9 as "slice 9b", on the assumption that the work
continued slice 9's `Bool`-as-enum change. It doesn't: `Bool`-as-enum shipped
(slice 9, merged `c5db035`) and is no longer a gate. What actually blocks
`if`-as-a-word is slice 10's row mechanism, whose representation and grounding
code this work directly extends, so it is renumbered into that lineage.

**Gates on 10a phases 1–2 only** (rows parsed and represented inside a nested
quotation effect, plus grounding at the splice site). It does **not** gate on
10a phase 3 (the self-tail back-edge rewrite: `if` has no back-edge) nor on 10b
(`times` moving to the library). The letter ordering implies more sequence than
exists; 10b first is a preference (shake the mechanism out on a real migration
before extending it to a second shape), not a dependency.

## Why the original signature couldn't work, and what replaced it

The first draft of this work targeted `if ( [ ..a -- ..b ] [ ..a -- ..b ] Bool -- ..b )`.
Two problems, both found by review rather than by implementation:

1. **`..a` has no top-level anchor.** It appears only nested inside two sibling
   quotation parameters, never bare in `if`'s own input list. 10a's decision 1
   ("a row inside a quotation effect must be the signature's own top-level row")
   therefore cannot admit it without inventing a second, quotation-scoped kind
   of row binding.
2. **The row genuinely changes shape** (`..a` in, `..b` out), where 10a's
   decision 2 requires the same row on both sides. `times`'s row is opaque and
   provably untouched; `if`'s is handed to a branch that transforms it.

The current direction fixes both by writing the rows at `if`'s own top level and
marking the branches inline-only:

```sooth
: if ( ..i Bool ~[ ..i -- ..o ] ~[ ..i -- ..o ] -- ..o )
```

`..i` is now `if`'s own top-level input row and `..o` its own top-level output
row, so decision 1 admits both as-is; decision 2 generalises from "the same row
on both sides" to "the quotation's input-side row is the signature's `row_in`,
its output-side row is its `row_out`" — a restatement, not new machinery. This
signature is also simply more honest: `if` really does transform an
unknown-sized region, and saying so beats letting it hide behind ordinary
"everything below my fixed inputs is invisible" semantics, which cannot express
a depth change.

## The `~` sigil, and why it moved into 10a

`~[ ... ]` marks a quotation that is **inline-only**: it has no runtime
representation, cannot be stored or returned, and cannot be reached by the
runtime `call` path. Its payoff here is not cosmetic:

- **It eliminates the erasure-boundary problem by construction.** A row-bearing
  quotation must never reach `materialize_quotation_at_boundary` or either
  `if`-join erasure site, because `QuotEffect` has no row field and a row's size
  isn't known at runtime. With `~`, that isn't a rule bolted onto three call
  sites — there is simply no coercion path from a `~` value to a runtime
  `Type::Quotation`.
- **It removes the need for abstract row-to-row unification.** Because a `~`
  value is always concrete by splice time, checking what a branch actually
  produces at a real call site is ordinary stack-effect checking, and the
  *existing* `if`-join equality check does the real verification: both branches
  are checked against the same grounded region and must agree.
- **It makes an invariant that is currently implicit into something declared.**
  A row-bearing quotation parameter *must* be spliced — that is a consequence of
  the row, not a design choice — so today's combinators rely on an unstated
  guarantee. `~` states it.

That last point is why `~` belongs in **10a**, not here: if every combinator
quotation parameter is inherently always-spliced-and-never-a-runtime-value, then
`~[ ..s i64 -- ..s ]` is the honest declaration for `times`'s own parameter too,
and shipping `times` with a type we would then have to change is the expensive
mistake. 10a owns the sigil; 10c consumes it.

**Open, and 10a's to settle** (recorded here because 10c inherits the answer):

- **What "cannot be used with `call`" means mechanically.** Either `call` is
  banned on a `~` value and invocation is by bare mention (with "push the
  quotation as a value" being impossible for a `~` type, so mention can only
  mean splice) — or `call` stays the invocation syntax and the ban is narrowly
  on storage/escape/runtime dispatch. The first is more in keeping with the
  language's preference for distinct syntax per mechanism and makes the cost
  visible at the use site; it also means rewriting `f call` throughout
  `lib/combinators.sth`. The second is a smaller change and keeps one concept.
- **How far `~` propagates.** Minimal is `times` alone. The logic leads further:
  `each`/`map`/`fold`/`filter` are combinators whose own quotation parameters
  likewise never become runtime values (each is spliced at its own call site,
  where its parameter becomes a literal), so `~` may be the honest type for all
  of them — making it the explicit marker of the combinator/closure boundary
  that currently exists only implicitly, with ordinary `[ ... ]` reserved for
  genuinely first-class capturing quotations (7b's territory). Larger blast
  radius, considerably more honest.

## What survives from the original brief, unchanged

Verified earlier and still true; none of it depends on the signature question:

1. **Clause dispatch is an independent primitive**, with its own
   discriminant-load lowering, not built on `if`. With `Bool` an enum (shipped),
   a clause-bodied word dispatching on it is exactly the mechanism every user
   enum eliminator already uses (`examples/shapes.sth`). No circularity.
2. **The guard blocking a quotation-taking clause body is stale.**
   `clause_bodied_quotation_word_error` rejects it, but its own comment names
   slice 7's runtime quotation value as the intended lift — and 7a shipped that.
   The rule was never revisited.
3. **Splicing must cover clause bodies, or the termination guarantee breaks.**
   `body_tail_calls_self` recurses into `TermKind::If`'s branches to find a tail
   self-call. Move those branches into quotation arguments of an un-spliced word
   and the self-tail → loop-back-edge transform stops firing — a real loss of
   tail position, not a detection gap. `examples/countdown.sth` (1M iterations)
   and `gcd.sth` (a Phase 0 golden, self-tail through `if`) both depend on it.
   So `is_combinator`/`collect_combinators`/`combinator_of` (today
   `WordBody::Terms`-only) and the alpha-rename pass must accept a clause body
   with a quotation parameter and splice the matching arm inline. **This is
   load-bearing, not an optimisation**: a naive un-spliced `if` cannot ship
   first and be optimised later, it stack-overflows a Phase 0 golden.
4. **Clause dispatch scrutinises the topmost input today** (`inputs.last()`),
   which must relax to "topmost *enum-typed* input" so the condition can sit
   deeper than the branches. For every existing clause word the two coincide, so
   this is additive.
5. **`cond`'s variadic form stays blocked on a different mechanism.** Clause
   dispatch is N-way on one scrutinee's variants; `cond` is N independent
   predicates evaluated in order. Different shapes. `cond` ships fixed-arity,
   written as nested `if`, whatever 10a delivers.

## What the spec must add, once 10a lands

- Re-verify every anchor against what 10a actually changed in `check_term`'s
  dispatch spine and the literal checks.
- Settle whether `if`'s own definition is checked standalone the way a
  combinator's body is (it has no self-reference, so its clause bodies are
  trivial), and if so, what grounds `..i`/`..o` in that no-caller context. The
  existing top-level row mechanism models the row as size-zero during a body
  check and rejects any body that touches it, so "the row transformed because a
  trusted call to a `~`-typed parameter says so" is new integration work, not an
  existing capability. Verified: `: shrinks-row ( ..a i64 -- ..b ) drop drop ;`
  fails with ``drop`needs 1 values, but the stack holds 0`.
- Whether `if` stays a compiler-recognised name with a real signature, or
  becomes genuine library source. The three surviving requirements above (2, 3,
  4) are needed either way.

## Out of scope

- `cond`'s true variadic `[ pred ] [ body ]`-list form: blocked on
  quotations-in-collections (slice 4's D4), independent of everything here.
- `Bool`'s representation: settled and shipped in slice 9.
