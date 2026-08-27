# Phase 7 Slice 8: nested inline-combinator splice-uid collision (brief)

Rewritten after two probe subagents (2026-08-27) refuted the original framing below.
**No generics are involved in the actual defect.** The original title ("poly-body-calls-
poly-combinator lowering") and the diagnosis under it are wrong; kept struck through for
the record, corrected version follows.

## Trigger

Found while trying to make `lib/cmp.sth`'s six surface comparisons (`eq`/`lt`/`gt`/
`lte`/`gte`/`ne`) `inline`, alongside `cmp` (already `inline` since S3s-follow). That
change was reverted; the comment in `lib/cmp.sth` above `impl: Ord for i8` still
documents it as blocked. This slice is the actual gap the revert names.

## The gap (confirmed by direct repro + gdb, no polymorphism required)

`impl: Ord for Point`'s own `cmp` body calling `lt`/`gt` internally — the ordinary,
idiomatic "derive `Ord` by delegating to a primitive comparison" pattern — panics once
those six comparisons are `inline`. Reproduces with a **concrete, non-generic** `main`
alone; `mymax` (the generic word in the original fixture) is not required and was
deleted in the repro without changing the outcome.

**Mechanism.** A bare trait-member call (`cmp`) inside an already-spliced combinator body
resolves through `resolve_splice_member_call` (`src/check/poly.rs:930`) to a concrete
`impl:` member symbol (e.g. `cmp;Ord;2;Point__m0`). That member's own body was checked
independently, as its own top-level word, under its own check-time splice-uid numbering
(seeded at `word_idx(cmp_for_point) * INLINE_UID_STRIDE`, `src/check.rs:1007`,
`INLINE_UID_STRIDE = 1<<20`, `check.rs:29`) — any ordinary combinator it calls internally
(here: `lt`, `gt`) got real `splice_trait_calls[(that_uid, span)]` entries recorded
against *that* numbering.

At lowering, `lower_resolved_word_call` (`src/ir/func_builder/calls.rs:187-195`) splices
that resolved member's body in place, and for its *own* top-level dispatch correctly
reuses the enclosing splice's uid (`self.splice_uid_stack.last()`, the `57943bb` fix —
this part works, one level deep). But any **ordinary combinator nested inside that
reused member body** — `Point::cmp`'s own `lt` call — goes through the *lowering
caller's* uid path (`calls.rs:189`, `self.splice_uid_stack.last()` again, but now this
is the outer caller's counter, not `Point::cmp`'s own check-time counter). The two
numberings are unrelated (traced exact values: check time assigns `Point::cmp`'s
internal `cmp`-inside-`lt`-inside-`gt` dispatch some uid from its own standalone check
pass; lowering re-splices it fresh at whatever the outer caller's `self.inline_uid`
counter happens to be, e.g. `1048577` vs the real `1048576`). The `(uid, span)` lookup
misses, falls through every dispatch path in `lower_call`, and panics at the literal
name `"cmp"`: `self.env.get(name).expect("checked user word exists")`
(`calls.rs:733`).

This is not latent on current `main`: today only `cmp` itself is `inline`+bound, and
nothing yet calls `cmp` a second time from inside an already-spliced `impl:` body.
Making `eq`/`lt`/`gt`/etc. `inline` is what first creates "an inline bound combinator
nested inside a reused trait-member splice" — a shape any user `impl: Ord for T`
delegating to a primitive comparison hits immediately, so this blocks the common case,
not a corner case.

**A second, related mismatch** (found independently by gdb tracing a `cross_calls_of`-
composed instantiation): when a poly word's cross-call to a bound `is_combinator`
callee is composed into a real monomorphized `IrFunc` (`cross_calls_of`,
`src/check/poly.rs:5666-5686`, `self.compose(...)`), the composed instantiation's
`FuncBuilder` is seeded with `inline_uid_seed = 0`, hardcoded at both lowering call
sites (`src/ir/driver.rs:365`, `:966`). That seed is correct in isolation (a fresh
instantiation's own body was never checked through a real, non-scratch `PolyCtx`, so a
`0` seed can't collide with *its own* check-time uids) but wrong the moment that
composed body transitively splices a concretely-checked combinator body (same
`lower_resolved_word_call` reuse path above) — the composed instantiation's `0`-based
counter collides with that concrete body's real, non-zero check-time uid namespace.
This is the same class of bug as the first one (a lowering-time uid that doesn't match
the check-time uid of a transitively-spliced, independently-checked body), reachable
through the generic composition path in addition to the non-generic member-splice
path.

## What was refuted from the original draft

- **"`check_poly_call` treats a poly-body call to a bound combinator as an ordinary
  generic call, with no splice and no rejection"** — false as a description of the
  actual failure. `mymax`'s own body is never walked by `check_poly_call` at all (it
  goes through the separate abstract `poly_walk`/`poly_cross_call`, `poly.rs:1326`/
  `2153`); grounding happens later in `cross_calls_of`, which **already has** an
  `is_combinator` branch that composes a real function for a bound combinator callee
  (`poly.rs:5666-5686`). This already works, independently verified: `mymax[i8]` and
  `mymax[i64]` both calling `gt` in one program, with no `impl:` involved, builds and
  runs correctly (two outer instantiations never collide — each `CallInst` carries its
  own `poly_calls: HashMap<Span, CallInst>`).
- **"No rejection path exists, so a located-error fallback is available"** — true that
  no rejection exists, but moot: the actual failing call site (`Point::cmp`'s internal
  `lt`) is issued from a **concrete, non-generic** caller. A guard scoped to "poly
  caller → bound combinator" would never fire on the real defect.
- **The `TraitCtx::scratch` "R9's scope cut" doc comment** (`poly.rs:50-51`) describes
  a historical design point with no live code behind it (`grep -rn
  reject_user_bound_on_combinator src/` — no matches) — accurate as a statement about
  dead code, but not evidence of what needs building; nothing here should be built as
  a rejection.

## Scope

Fix the uid-reuse mismatch at its two confirmed sites:

1. **`lower_resolved_word_call`'s combinator branch** (`calls.rs:189`): splicing a
   resolved member's body must lower any *further* nested combinator splice inside
   that body under the member's own check-time uid namespace, not the enclosing
   caller's. Requires threading the resolved member's own uid (or enough to
   reconstruct it — `word_idx(member) * INLINE_UID_STRIDE`) alongside `sym_name` into
   `lower_resolved_word_call`, and pushing it onto `splice_uid_stack` for the duration
   of that body's lowering rather than reusing `.last()`.
2. **Composed-instantiation seeding** (`driver.rs:365`, `:966`): `inline_uid_seed = 0`
   is wrong whenever the composed body transitively reaches a concretely-checked
   combinator body via the same reuse path. Either give composed instantiations their
   own disjoint uid stride (a third numbering axis alongside `word_idx *
   INLINE_UID_STRIDE`), or make the reuse path in (1) uid-correct so this stops
   mattering — probe which; they may collapse to one fix.

## Open questions, not yet probed

- **Self-tail recursion** (a recursive poly word calling a bound combinator in its
  tail branch, interacting with `emit_back_edge`/`cur_combinator`): genuinely unknown,
  flagged by the dogfood probe as needing its own spike before this slice's exit
  criteria can claim it.
- Two different bound combinators called in sequence from the same body: low-risk
  extrapolation from the working shape-1 case (distinct spans, no reuse-uid
  indirection), not independently verified.

## Out of scope

- Making `lib/cmp.sth`'s six comparisons `inline` is *not* required by this slice —
  that is a one-line follow-up once the uid fix lands, done as its own tiny commit.
- Any change to `is_combinator`'s definition, `check_poly_call`, or the P7.S3o
  concrete-caller splice path's happy path — all confirmed working, out of this
  slice's blast radius.
- A rejection-based fallback design: refuted above as not applicable to the real
  defect.

## Exit

`cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green with
`lib/cmp.sth`'s six comparisons made `inline`; `an_ord_bounded_generic_word_
instantiates_over_a_user_struct` and a new, `mymax`-free regression test (`impl: Ord
for T` delegating to `lt`/`gt` directly, no generics anywhere in the call chain) both
pass; the self-tail-recursion open question above is either resolved or explicitly
re-scoped out with a located test proving it's rejected rather than silently
miscompiled.
