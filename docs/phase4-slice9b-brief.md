# Phase 4 Slice 9b: `if`/`cond` as pure Sooth words (brief)

Split out of slice 9 after slice 9's original P3–P5 turned out to be mis-specified:
the phase that was meant to make `if` an ordinary clause-bodied word could not
possibly have succeeded, because the word's own signature does not parse. This
brief exists so that finding, and the real design work that survives it, aren't
re-discovered from scratch once the blocker clears.

**Blocked on ROADMAP slice 10a. Do not spec this until 10a is merged.**

## What actually blocks it (verified against the tree, twice)

A row variable works at a word's own top level but not inside a nested
quotation's declared effect:

```sooth
: keep ( ..a i64 -- ..a i64 ) ;              \ fine
: apply2 ( [ ..a -- ..b ] -- ..b ) call ;    \ error: unknown type `..a` at line 1, col 14
```

`if` genuinely needs a row here, not a type variable or a family of concrete
overloads: its two branches must agree on "consumes some prefix, leaves some
other prefix" for *arbitrary* prefixes, and the corpus alone needs at least four
incompatible concrete shapes (`( -- i64 )` for `gcd`/`factorial`, `( 'T 'T -- 'T
)` for `mymax`, `( 'a -- 'a )` for `while`, `filter`'s cursor-threading shape). A
type variable abstracts over *types*, not over *how much stack a branch
touches*; a family of overloads doesn't compose either, since two candidates
differing only in a declared quotation's effect are already recorded as
indistinguishable to resolution in 8a's own spec (the first declared wins
silently). `[ ..a -- ..b ]` is the only notation that says what's actually
needed, and parsing/checking it inside a quotation's declared effect is exactly
ROADMAP slice 10a's mechanism — deliberately deferred by slice 6a's R2/R28, which
is also why `times` is a compiler intrinsic today rather than a library word.

This was found *after* slice 9's P3 had already been (badly) attempted and the
target `pick`/`Choice` proof test turned out to be unwritable — the blocker is in
the spec, not in what any implementer did with it. See the project memory note
`project_rows_in_quotation_effects_blocker` for the repro.

## What is *not* blocked, and survives into the next spec unchanged

Everything about `if`'s **dispatch** half was verified sound and stays true once
10a lands:

1. **Clause dispatch is a real, independent compiler primitive** (its own
   discriminant-load lowering in `ir.rs`), not built on top of `if`. Once `Bool`
   is an enum (slice 9, shipped), an ordinary clause-bodied word dispatching on
   it is exactly the mechanism every user enum eliminator already has
   (`examples/shapes.sth`'s `area`/`unwrap-or`). No circularity.
2. **The guard actually blocking a quotation-taking clause body is stale.**
   `clause_bodied_quotation_word_error` (`check.rs`, fired near a check on
   `WordBody::Clauses` + a quotation-typed input) rejects it, but its own comment
   says the intended lift is "slice 7's runtime quotation value... the word would
   then `call` a real value, no inlining needed." Slice 7a shipped that value;
   this rule was never revisited. Lifting it is real but small.
3. **Splicing has to cover clause bodies too, or the termination guarantee
   breaks.** `body_tail_calls_self` (`ir.rs`) recurses into `TermKind::If`'s
   branches to find a tail self-call; move the branches into quotation arguments
   of an *un-spliced* word and the self-tail → loop-back-edge transform silently
   stops firing — not a detection gap, a genuine loss of tail position, since the
   call now returns into a closure that returns into `if`'s body. `examples/
   countdown.sth` (1M iterations) exists specifically to prove this transform
   works, and `gcd.sth` — a Phase 0 golden — is self-tail recursive through `if`
   in the same shape. So `is_combinator`/`collect_combinators`/`combinator_of`
   (currently `WordBody::Terms`-only) and the alpha-rename pass need to accept a
   clause body with a quotation parameter and splice its matching arm inline
   (discriminant test + inlined arm, no `Instr::Call`), and the self-tail
   detector needs to see through that splice. **This is the load-bearing
   mechanism, not an optimisation** — a naive/un-spliced `if` cannot ship first
   and be optimised later; it stack-overflows a Phase 0 golden.
4. **Clause dispatch scrutinises the topmost input only, today** — `check_clause_word`
   hard-codes `word.effect.inputs.last()`. Factor-order `cond [ then ] [ else ]
   if` needs `Bool` deepest of three, so this has to relax to "topmost
   **enum-typed** input" (in the checker and the mirroring clause lowering) or
   `if`'s call order has to bury the condition, breaking DESIGN.md's documented
   idiom. For every existing clause word the topmost enum-typed input already is
   the topmost input, so this is additive, not a behaviour change for anything
   that exists.
5. **`cond`'s variadic form stays blocked, on a genuinely different mechanism** —
   this was a real error in slice 9's own first draft, corrected in-spec before
   anything shipped on it: generalising clause splicing does **not** give
   variadic `cond` for free. Clause dispatch is N-way dispatch on *one
   scrutinee's variants*; `cond` is N *independent boolean predicates* evaluated
   in order. Different shapes. `cond` ships fixed-arity, written as nested `if`,
   regardless of what 10a delivers.

## What the next spec needs to add, once 10a lands

- Confirm 10a's row mechanism actually covers a *two-branch, shared-effect* shape
  (`[ ..a -- ..b ] [ ..a -- ..b ]`), not just `times`'s single-quotation
  `( ..s i64 [ ..s i64 -- ..s ] -- ..s )` — don't assume it transfers, check it
  the way this brief's predecessor should have checked the signature parses at
  all.
- Re-verify every anchor above against whatever 10a actually changed in
  `check_term`'s dispatch spine (10a is explicitly sequenced to land after 7b and
  8a "since all three touch" it).
- The fork this brief leaves open: whether `if` ships as the pure clause-bodied
  word described here, or stays a name-recognised intrinsic (which needs neither
  10a nor the splice generalisation) is a real design call, not a foregone
  conclusion — write it up as an open question, not inherited from this brief as
  settled.

## Out of scope (unchanged from slice 9's original framing)

- `cond`'s true variadic `[ pred ] [ body ]`-list form: blocked on first-class
  quotations-in-collections (slice 4's D4 rejects a quotation as an array
  element), independent of 10a.
- Everything about `Bool`'s representation — settled and shipped in slice 9.
