# Phase 4 Slice 6g — combinator splices bypass 6f's granting rule (brief)

6f built a mechanism (`Liveness`'s `outer_releasable`/`back_edge`, `releasable_into`) that
lets a nested invocation — an `if` arm, a `call`'d quotation body, a `times` body — learn
that the caller has already proven an ancestor-bound name has no residual use past this
point, so that name can die inside the nested body instead of defaulting to "never dead
here" (6f's own words). `inline_combinator` (`src/check.rs:7194`), the function that
splices a combinator's body (`filter`/`map`/`fold`/`while`/any user quotation-taking word)
at its call site, is a fourth nested-invocation shape but was never wired into this
mechanism: it calls the plain `check_terms` (`src/check.rs:8037`) — the entry point 6f's own
doc comment reserves for "a word body, a REPL line, a `case` clause: nothing is ancestor to
those" — instead of `check_terms_relaxed` with a computed `outer_releasable` set. This
slice closes that gap. Everything below was run, not read.

## Recon: measured against the built compiler

**1. The bug reported and deferred at 6b's own recon 10 has a single, confirmed root
cause.** 6b's brief named it "a pre-existing 6a inliner limitation... whether to fix the
underlying alias-after-move tracking is 6a's business, not this slice's" and moved on
without root-causing it. Minimal repro, unchanged since 6b, still reproduces verbatim on
the current compiler:

```
0 4 fill | a |
a [ 4 > ] c::filter drop drop
→ error: cannot borrow `arr__inl0` mutably: it is aliased by `a`
```

**2. Every array in Sooth is `Copy`, so naming one never enters move-tracking at all.**
`fill` is the only array constructor and rejects a linear element outright
(`src/check.rs:6620`/`:2017`) — confirmed again here, not just cited from 6b's recon 8. A
`Copy` local is, by `Moves`' own doc comment (`src/check.rs:920`), "absent from the map":
`Moves::take` (`:929`) returns `Ok(())` for a name it has never seen and never inserts one.
So `scope.moves.moved_site("a")` is `None` forever, regardless of how many times `a` is
named and consumed. `aliasing_origin`'s (`:1594`) filter `scope.moves.moved_site(&b.name).
is_none()` — meant to exclude an already-consumed rival name — can never exclude an array
this way. `Liveness::dead` is the only remaining guard.

**3. `check_terms` vs `check_terms_relaxed` is the actual fork, and `inline_combinator` is
on the wrong side of it.** `times` (`:8394`), `call` (`:8308`), and an `if` arm (`:8797`)
all compute `releasable_into(scope, base_depth, outer_releasable, &siblings[at + 1..])`
and pass the result into `check_terms_relaxed`. `inline_combinator`'s own body-check
(`:7300`, the tail of the function, shared by every combinator regardless of `comb.word.
poly`) calls plain `check_terms`, which hardcodes `outer_releasable = HashSet::new()` and
`back_edge = false` (`:8037`). Traced by reading, then confirmed by the repro above:
`arr__inl0`'s mutable borrow calls `aliasing_origin`, which finds `a`'s alias set overlaps
and checks `!live.dead("a", at) || captured.contains("a")` — `live` here is
`Liveness::scan`'d fresh over just the *spliced* body's own term list (`:8094`), which never
mentions `a` at all, so `dead()` falls to its `None` arm (`:1304`): `self.outer_releasable.
contains("a")`, which is always false, because `inline_combinator` never populated it.

**4. The same mechanism reproduces without `filter`, without genericity, and down to a
single no-op iteration.** Confirmed independently (three separate from-scratch minimal
repros, each isolating one variable):

- Nesting `c::while` inside a `times` loop that rebinds/swaps array-typed row items
     across iterations, borrowing both a shared and a mutable reference inside the `while`'s
     quotation.
- Splitting merge-style logic into a separate, non-combinator, non-generic word taking a
     quotation parameter, called from that same swapping loop.
- Neither the swap, the loop nesting depth, nor `'T`-genericity is what triggers it —
     every variant above reproduces with fully concrete (`[i64 3]`, hardcoded comparator)
     types and a single outer iteration. What is common to every triggering shape and absent
     from every non-triggering one tried is exactly recon 3's mechanism: a combinator splice
     (`while`, or any quotation-taking word call) sitting between the caller's array local and
     the point it gets borrowed again.
   `lib/arrays.sth`'s `sort` (in-place bottom-up merge sort) currently works around this by
   inlining all of its merge logic into one word body and using a fixed-bound `times` instead
   of `while` for its innermost loop — both forced, documented in the file's own header
   comment, not stylistic.

**5. `inline_combinator`'s call to `check_terms` is a single, shared exit point for both
monomorphic and polymorphic combinators.** `comb.word.poly.is_some()` only branches the
*argument*-side check (`check_poly_combinator_args` vs the inline mono loop, `:7213`–
`:7273`); the body splice at the end of the function (`:7300`) runs unconditionally for
both. `while` itself is polymorphic (`'a [ 'a -- 'a bool ] -- 'a`), and recon 4's `while`
repro exercises exactly this shared path — so one fix site covers both the mono (`filter`/
`map`/`each`/`fold`) and poly (`while`, any user poly combinator) cases; there is no second,
poly-specific splice path this bug also lives in.

**6. `inline_combinator` does not currently have access to `siblings`/`at`/`base_depth`/
`outer_releasable`.** It is called from exactly one site (`:8640`, inside `check_term`'s own
`TermKind::Call` dispatch), which *does* have all four in scope — the same values `times`/
`call`/`if` already use at their own `releasable_into` call sites, a few hundred lines away
in the same function. Threading them the last few lines into `inline_combinator` is
mechanical.

**7. The three existing `releasable_into` call sites disagree on `back_edge`, and the
reason tracks whether the *body being spliced* can run more than once, not whether the call
*site* is textually reachable more than once.** An `if` arm (`:8797`) is `back_edge =
false`: it is a single, execute-once splice, "may die at its own last use inside" (comment
at `:8794`). `times` (`:8394`) is `back_edge = true`: the same one splice runs N times at
runtime. `call` (`:8308`) is also `back_edge = true`, despite splicing a *literal* body at
one textual site — the comment there (`:8302`, "a quotation body can be called from
elsewhere too") gives the reason: the `Known` quotation value is `Copy`, so the *same*
literal could be bound to a local and invoked again from a different call/times/combinator
site the checker cannot see from here, and the conservative choice is to treat any granted
name's use anywhere in the body as pinning it live throughout, rather than at its last
syntactic use. A combinator's body is not itself a `Copy`, reachable-from-elsewhere value in
this sense: it is spliced fresh, once, at this exact call term, every time this term is
reached. The candidate rule — `back_edge = self_tail` (`:7280`, already computed in
`inline_combinator` for the loop-back-edge marker), `false` otherwise — treats a plain
combinator splice like an `if` arm (one shot) and a self-tail combinator's splice like
`times` (its own body re-enters via the back-edge). Not yet confirmed against the compiler
either way; see open questions.

## Decided (locked)

**D1. The fix is `inline_combinator`'s final body-check call: `check_terms` becomes
`check_terms_relaxed` with a `releasable_into`-computed `outer_releasable` set.** Forced by
recon 3 and 6, not chosen — this is the one call site on the wrong side of 6f's own
`check_terms`/`check_terms_relaxed` contract, and the other three correct call sites are the
worked pattern to copy.

**D2. No change to `Moves`, `aliasing_origin`, or the Copy-array move-blindness itself.**
Recon 2 is a true, permanent, and correct property of the language (Copy locals are not
move-tracked by design — `Moves`' own doc comment states this as the intended shape, not a
gap), not a bug to fix. It is only a problem in combination with recon 3's missing
`outer_releasable` wiring; fixing recon 3 alone closes the repro without touching move
semantics.

**D3. `lib/arrays.sth`'s `sort` header comment documenting the inline-everything/no-`while`
workaround is deleted once the fix lands and the workaround's rationale no longer holds.**
Whether `sort`'s merge logic is actually *restructured* to use `while`/a separate word is
not required by this slice — only that the workaround's stated reason for existing is
retested and found false.

## Open questions the spec must answer

- **The exact `back_edge` value for a combinator splice.** Recon 7's candidate rule
  (`self_tail` for a self-tail combinator, `false` otherwise) is reasoned from the existing
  three call sites' own stated justifications, not yet built and tested. The spec must
  probe: does a self-tail combinator (`while`) need `back_edge = true` to avoid a *new*
  false rejection (a granted name used only once, on an early loop iteration, wrongly dying
  before a later iteration's use) the way `times`'s own loop body does? Does a plain,
  non-recursive combinator (`filter`) genuinely behave like an `if` arm (`false`) with no
  observable regression, or does treating it as `false` under-grant in some shape recon
  did not try (e.g. a combinator call sited *inside* another loop, legal since 6d)?
- **Whether `inline_combinator` should take `granted: &HashSet<String>` computed by its
  caller (mirroring `call`/`times`, which compute `releasable_into` inline at their own call
  site before recursing) or take `siblings`/`at`/`base_depth`/`outer_releasable` directly and
  compute it internally.** The former keeps `inline_combinator`'s signature smaller and
  matches the existing pattern exactly; the latter avoids a redundant `releasable_into` call
  if `inline_combinator` is ever invoked from a second site in the future. Recon 6 found only
  one call site today, so this is a style choice with no measured consequence — the spec
  should just pick one and say why.
- **A mutation test for the fix, not just a green build.** Per project convention (the
  alias half of 6f's own slice was "mutation-tested rather than merely run green" because a
  wrong answer here is a silent wrong *value*, not just a rejected program) — but this bug's
  failure mode is the opposite: a false *rejection*, not a false acceptance. Confirm the test
  shape: reverting `inline_combinator`'s call back to plain `check_terms` must make the
  recon-1 minimal case start failing again, and the spec should say whether that reversion is
  itself the mutation test or whether a `#[cfg(test)]` unit test closer to `inline_combinator`
  is also warranted (comparable to 6f's own R6 mutation-test precedent in
  `docs/phase4-slice6f-spec.md`).
- **Whether any existing golden/example changes shape.** `examples/filter_while.sth`'s own
  comment states it deliberately routes arrays through producer words "so this does not trip
  6a's bind-then-pass alias limitation" — once this slice lands that avoidance is no longer
  necessary. The spec should decide whether to leave that example as-is (it still compiles
  and its comment becomes historical) or add a second example/golden that deliberately binds
  first, to positively pin the fix rather than only removing an old workaround's necessity.

## Out of scope

Rewriting `lib/arrays.sth`'s `sort` to actually use `while` or a split-out merge word (D3
only requires the stale rationale be deleted, not the code be restructured). Any change to
`Moves`/move-tracking semantics for `Copy` types (D2). The polymorphic-body `if` gap
(`src/check.rs:3664`/`:3672`) and the polymorphic self-call gap (`poly_call_term`, unrelated
— `while`'s repro goes through `inline_combinator`'s shared splice tail, never through
`poly_call_term` at all, since a combinator's own call resolution is intercepted earlier by
`poly.combinators.get(name)`, `:8629`). The `PolyType::Ref` gap (a separate, already-scoped
defect: `&name` unparseable at a generic word's own top-level body — orthogonal borrow-sigil
work, not this slice's). Sequencing: after slice 10a lands, before 10b/10c begin (ROADMAP.md
Phase 4 item 6g states the reason: both widen this exact bug's blast radius by turning a
compiler-intrinsic control-flow form into a combinator splice).
