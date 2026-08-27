# Phase 7 Slice 8: poly-body-calls-poly-combinator lowering (brief)

## Trigger

Found while trying to make `lib/cmp.sth`'s six surface comparisons (`eq`/`lt`/`gt`/
`lte`/`gte`/`ne`) `inline`, alongside `cmp` (already `inline` since S3s-follow). That
change was reverted; the comment in `lib/cmp.sth` above `impl: Ord for i8` still
documents it as blocked. This slice is the actual gap the revert names.

## The gap

`is_combinator` (`src/check/combinators.rs:155`) is `word.declares_inline` alone —
no requirement that the word take a quotation parameter, and no exclusion for a
polymorphic signature. So a word can be **both** a combinator (spliced at every call
site, mints no `IrFunc`) **and** generic with a trait bound (`'T: Ord`), which is
exactly what `eq`/`lt`/etc. would be if inlined: `( 'T: Ord 'T -- Bool )`, `inline`.

Calling such a word from a **concrete** caller already works (`check_poly_combinator_args`,
the P7.S3o stand-in machinery, covers it — see the passing
`ord_inline_cmp_two_splices_produce_correct_value` and siblings in
`tests/phase7_slice3s_flip.rs`). The gap is calling it from **inside another
polymorphic word's body**: `check_poly_call` (`src/check/poly.rs:4919`) is the only
path a poly body's call to another poly word goes through, and it does not
special-case `is_combinator(callee)` at all — it records an ordinary `CallInst`
(`poly.insts` or, inside a splice, `poly.splice_records`) exactly as it would for a
non-inline generic word, and moves on. Nothing splices the callee's body into the
caller's poly body; nothing rejects the call either. The scratch `TraitCtx`'s own doc
comment (`src/check/poly.rs:50-51`) says this combinator case is supposed to be "a
located rejection" — but the guard that comment describes (a
`reject_user_bound_on_combinator`, referenced only in stale session notes) was never
built; P7.S3o parked it as unsound in two prior design attempts.

**Observed failure**, reproduced with `eq`/`lt`/`gt` inlined and this fixture
(`tests/phase7_slice3s_flip.rs`'s `an_ord_bounded_generic_word_instantiates_over_a_user_struct`):

```forth
: mymax ( 'T: Copy Ord 'T -- 'T )
  | a b | a b gt ~[ a ] ~[ b ] if ;
```

`mymax` is generic; its call to `gt` (generic, `inline`) checks fine (`check_poly_call`
records an instantiation for it, same as any generic callee) but lowering never
finds a splice or a monomorph for it: `src/ir/func_builder/calls.rs:733`,
`self.env.get(name).expect("checked user word exists")` panics, because a
combinator (per `INV-INLINE-COMBINATOR`, `src/check/combinators.rs`) mints no
`IrFunc` and so has no `env` entry, and no `(uid, span)`/span-keyed splice record
covers "a poly body's call to a poly combinator" either — that machinery
(`poly.splice_records`, `src/check/poly.rs` P7.S3o) was built for a *concrete*
caller splicing a poly combinator, not a poly caller splicing one.

This is not a corner case once comparisons are the combinator: `'T: Ord` generic
code calling `<`/`>`/`==`/etc. from inside another generic word is the ordinary
shape of generic comparison-using code, not an edge case.

## Scope

Two shippable outcomes, either is a valid exit for this slice on its own, but the
splice is the one that actually unblocks inlining `eq`/`lt`/etc.:

1. **Splice** — a poly body's call to an `is_combinator` word with a `Bound::User`
   or `Bound::Copy` on its own type variables lowers by splicing the callee's body
   into the caller's poly body, under the caller's own concrete instantiation
   substitution composed with the callee's substitution — the same shape
   `check_poly_combinator_args`/`resolve_splice_member_call` already give a
   *concrete* caller, extended to a *poly* caller. The uid-keying
   (`splice_uid_stack`, `splice_records`) that already disambiguates two splices of
   the same combinator at different concrete types needs to additionally
   disambiguate two splices reached through two different *outer* poly
   instantiations (`mymax[i8]` vs `mymax[Point]`, say) — probe whether the existing
   `(uid, span)` key already covers this (a fresh `uid` per outer monomorphization's
   own splice) or needs a third axis.
2. **Reject** — finish the P7.S3o-parked `reject_user_bound_on_combinator`: a
   located error at the *call site* (not an ICE) when a poly body calls an
   `is_combinator` word whose own signature carries a bound. Cheaper, but means
   `eq`/`lt`/etc. staying `inline` makes them *uncallable* from any generic Ord-bounded
   word — probably not the outcome anyone wants, so treat this as a fallback if (1)
   turns out to need a design neither prior P7.S3o attempt found sound, not the
   default target.

## Prerequisite work

- Read the two prior P7.S3o design attempts (session notes reference them as
  "unsound," no detail retained) before re-attempting (1) — this slice should not
  re-discover the same dead ends.
- `tests/phase7_slice3s_flip.rs`'s `an_ord_bounded_generic_word_instantiates_over_a_user_struct`
  and `an_unsatisfied_ord_bound_names_the_missing_impl` are the existing goldens
  for this exact shape (currently pass because `gt`/`lt` are non-inline); flipping
  `lib/cmp.sth`'s six comparisons to `inline` as part of this slice's own dogfood is
  the regression check.

## Out of scope

- Making `lib/cmp.sth`'s six comparisons `inline` is *not* required by this slice —
  that is a one-line follow-up once (1) or (2) lands, done as its own tiny commit so
  the `lib/cmp.sth` comment and the mechanism it depends on land in the same review.
- Any change to `is_combinator`'s definition (still `declares_inline` alone) or to
  the P7.S3o concrete-caller splice path, both of which already work and are out of
  this slice's blast radius.

## Exit

`cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green with
`lib/cmp.sth`'s six comparisons made `inline` (if (1) is chosen) or with a located
diagnostic replacing the `checked user word exists` ICE at the call site (if (2) is
chosen), and `tail_splice_check_and_lowering_agree_on_the_loop` (whose id-ordering
loop-detection heuristic was fixed to real graph-cycle detection while finding this
gap, `src/ir/driver.rs`) stays green under the added combinator-body shape.
