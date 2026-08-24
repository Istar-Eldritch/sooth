# P7.S3s brief — `Ord` as a library trait, not a compiler-hardcoded bound

## Problem, confirmed live against current `main`

`Bound` (`src/ast.rs:1679-1683`) has three variants: `Copy`, `Ord`, and `User(TraitId)`.
The first two are **reserved, member-less trait-table entries**
(`seed_predicate_traits`, `ast.rs:1793-1811`) that exist only so `parse_capabilities`
(the `'T: Copy Ord` parser) can look every bound name up through one uniform mechanism.
Satisfaction is never nominal for either: `poly_is_copy`
checks a closed set of concrete-type predicates (`is_copy`), and `is_ord`
(`src/check/poly.rs:120-122`) is exactly:

```rust
pub(super) fn is_ord(ty: Type) -> bool {
    ty.is_numeric()
}
```

— a hardcoded numeric-tower check, consulted at every `Bound::Ord` discharge site
(`declarations.rs:1360`, `poly.rs:1881`, `poly.rs:4428`, `poly.rs:4693` — line numbers
drift as active files churn; note that only two of these four are actually
bound-satisfaction sites, see "the overload-admission filter" below). None of these
sites ever consult the `impl:` registry `Bound::User` dispatch already uses
(`check_impl_decls`, `declarations.rs:407-482`; whole-program `(TraitId, Type)` lookup,
`slice3e-spec.md` design ruling 9).

**There is no live footgun here — both reachability paths are cleanly closed today**,
probed against current `main`. An earlier draft of this brief speculated that
`impl: Ord for Point` might type-check while doing nothing, on the reasoning that a
`Predicate`-kind `TraitDecl` carries `members: Vec::new()` and so satisfies
`check_impl_decls`'s missing-member loop vacuously. It does not get that far:

```
$ impl: Ord for Point ...
error: trait `Ord` cannot be implemented at line 5, col 7
  (it is a built-in predicate, satisfied by a type's own shape)

$ trait: Ord 'T ...
error: trait `Ord` (line 3, col 8) is already the name of a trait in this module
```

Both are correct, located rejections. The gap is a missing capability, not a silent
misbehaviour. **Consequence for this slice:** the built-in-predicate implementation guard
must be narrowed to `Copy` alone when `Ord` stops being a predicate, and the reserved-name
collision must stop applying to `Ord` (or `Ord`'s seeded declaration must itself become
the library one). Both are phase-2 work, and both need a test that the guard still fires
for `Copy`.

The consequence for a user: `'T: Ord` bounds a struct or enum out categorically. `sort`,
`bin_search`, or any comparison-bounded generic word can only ever be instantiated over
the numeric tower, by construction — and a user cannot opt their own type in, since the
`impl:` that would do it is rejected outright (above). This is the gap `Program 2` of `docs/roadmap/P7/slice3-dogfood.md`
(cited by `slice3e-spec.md`'s own "Forcing consumer" note) worked around by inventing a
*separate*, user-declared `Order` trait (`examples/traits.sth`, `cmp ( &'T &'T --
Ordering )`) rather than using the language's own `Ord` — because the language's own
`Ord` cannot be satisfied by a struct at all.

## What already works, confirmed

- **`impl:` for a primitive scalar type already works.** `impl_target_module` returns
  `None` for any non-struct/non-enum `Type` (`declarations.rs:394-400`), which the orphan
  rule (`declarations.rs:432-438`) treats as "no home module, so must live in the trait's
  own module" — not a rejection. Pinned by
  `check_impl_decls_orphan_scalar_target_names_only_the_trait_module`
  (`declarations.rs:3488-3507`). So `impl: SomeTrait for i64` inside the trait's own
  declaring module is already legal.
- **A binary comparison member's receiver position is fine as-is.** `Order.cmp ( &'T
  &'T -- Ordering )` (`examples/traits.sth`) binds its variable in an input, which is all
  `member_binds_trait_var` (`declarations.rs`) asks, and `poly_trait_member_call`
  dispatches off the bound rather than off the top of the stack, so position is no
  constraint at all — **this slice does not depend on receiver position** for a
  `cmp`-shaped `Ord`, and a later shape like `clamp ( &'T lo:'T hi:'T -- 'T )` with a
  non-trailing receiver would dispatch too (P7.S3p).
- **`lib/cmp.sth` already exists and is real, not a P8-planned dogfood sketch.** `eq`,
  `lt`, `gt`, `lte`, `gte`, `ne` are `'T: Copy Ord`-bounded `inline` words over the six
  raw comparison intrinsics (`ueq`/`ult`/.../`une`) and `bool`'s `branch`. (The identical
  file also exists at `docs/roadmap/P8/dogfood/core/cmp.sth` as a planning artifact —
  the two should be checked for drift before this slice touches either.)
- **P7.S3k (generic-calls-generic) is closed**, landed and merged (`92e7391`, `6115c86`,
  `4c8e6b5`). A non-`inline` generic word calling `Ord`-bounded library comparisons is no
  longer blocked by that gap. (Superseding stale project memory that called this "closed
  in halves" — re-verify against `main` before relying on this, per that memory's own
  caveat.)

## The design, decided

`Ord` becomes an ordinary library trait. No predicate variant, no compiler-level
numeric-tower knowledge:

```sooth
type: Ordering | Less | Equal | Greater ;

trait: Ord 'T
  cmp ( &'T &'T -- Ordering )
;
```

`impl: Ord for i64` and one per remaining numeric width are written in `core`, each body
built from the raw comparison intrinsics (`ult`/`ueq`/...) exactly as `lib/cmp.sth`'s
bodies are today. The six surface comparisons (`lt`/`gt`/`eq`/`lte`/`gte`/`ne`) are then
derived from `cmp` rather than from the intrinsics directly, and `Bound::Ord` is deleted
from the `Bound` enum.

**`&'T &'T -- Ordering`, not `'T 'T -- Ordering`.** Two by-value operands consume both,
so a `sort` cannot compare the same element twice. `Order` in `examples/traits.sth`
already uses borrows for this reason.

### The blocker, probe-confirmed: generic-calls-generic under a user bound

Once `lt` is a `'T: Ord`-bounded library word, every generic consumer of it (`sort`,
`bin_search`, any user word) is a polymorphic body calling a polymorphic word whose
signature carries a `Bound::User` — which is rejected outright today:

```
error: `pick` cannot call the polymorphic word `is_less`
  discharging the `Order` bound is not yet supported from a polymorphic body
```

`Bound::Ord` is exempt today only because it is a *predicate*: a predicate needs no
instantiation record to resolve against. This is the structural reason `Ord` was
hardcoded in the first place, and it is why moving it to the library is not a pure
library change.

**The checker side is already written.** The symbolic forwarding arm
(`poly.rs:1856`) is bound-agnostic — `(Image::CallerVar(t), _) => !sig.has_bound(t,
bound)` — so a caller forwarding its own identical bound to a callee is handled
correctly for `Bound::User` already. It is merely unreachable behind the blanket
rejection in `poly_cross_signature_supported` (`poly.rs:2188`). Probed by deleting only
that rejection: the forwarding program **type-checks**. It then ICEs in lowering at
`calls.rs:737` (`checked user word exists`) — the callee's trait-member obligation is
never resolved to a concrete symbol per instantiation on the cross-call path. That
lowering gap is the substance of this slice.

### Scope: the comparisons drop `inline`, and that is deliberate

`is_combinator(word)` is exactly `word.declares_inline` (`combinators.rs:155-157`) —
a quotation parameter is irrelevant. All six comparisons in `lib/cmp.sth` are `inline`
today, so a `'T: Ord`-bounded `lt` would hit `reject_user_bound_on_combinator` on its own
declaration. That rejection is **P7.S3o**, parked with two designs found unsound in
review, whose failure mode is silent dispatch to the wrong `impl:`.

This slice therefore ships the six comparisons **non-inline**. Each becomes a real
monomorphized call frame, and QBE performs no cross-function inlining, so a comparison
that is one `ult` today becomes a leaf call. That is a genuine, pervasive regression
against today's codegen, accepted for one slice's duration on the following reasoning:

- The cross-call lowering gap must be closed either way. `bin_search_internal`
  (`examples/experiments/binary_search.sth`) is a non-inline `'T: Ord` word today and
  user code will write more, so S3o does not subsume this work. This slice is a *prefix*
  of the total work, not a detour.
- Attempting S3o under schedule pressure from this slice biases the design toward a
  resolution key that happens to work for `lt`, rather than one sound in general — which
  is the precise shape of both prior failures.
- Landing non-inline first hands S3o the oracle it has never had: a correct
  implementation to **differential-test** the spliced version against. Flip `inline` on
  the same source, diff program output and the resolved `impl:` symbols (`nm`), at two
  splices, at three, and inside a materialized quotation literal. That converts S3o's
  untestable soundness property into a mechanical diff, answering the methodological gap
  round 2 identified directly.

### Also in scope: the overload-admission filter

`poly_admits` (`declarations.rs:1360`) and `poly_sig_could_match` (`poly.rs:4428`) are
*not* bound-satisfaction sites — they are slice 10c's overload-admission filter, using
"has an `Ord` bound && `!is_numeric` → decline" to keep the library's generic `lt` from
swallowing a user's own concrete `Vec2 lt`. With `Bound::Ord` deleted, that filter goes
dead and the generic `lt` admits every operand type again. Both sites must become a real
`(TraitId, Type)` registry lookup: decline unless the operand type has an `impl: Ord`.

### Diagnostics

`poly_ord_bound_error` and `poly.rs:2285`'s `Bound::Ord => "Ord"` naming disappear with
the variant. An unsatisfied `Ord` becomes an ordinary user-trait failure, and should name
the missing `impl:` the way a `Bound::User` failure does.

## Dependencies / sequencing

- **Blocks nothing on P7.S3o, but hands it its entry condition.** S3o's brief parks it
  with "revisit only if a concrete program actually needs bound dispatch on a
  combinator's own type variable". This slice is that program. S3o becomes the named
  follow-on: re-`inline` the six comparisons, with the differential harness above as its
  entry condition and its own benchmark to justify itself against.
- **No dependency on P7.S3p.** `cmp`'s receiver is trailing; the non-trailing fix is not
  needed.
- **No dependency on P7.S3k**, already closed.
- **Sequence before P8.S2's `lib/cmp.sth` migration.** P8.S2 moves the surface
  comparisons out of the compiled-in prelude into a gated-import module, still
  `'T: Copy Ord`-bounded as today. This slice changes what `Ord` *is* and what those
  bodies are built from, so P8.S2 must be written against the post-S3s shape or one of
  the two gets redone.

## Exit criteria

- `Ord` bounds a struct or enum, satisfied nominally by an `impl: Ord for Point` block —
  a comparison-bounded generic word (`sort`, `bin_search`) instantiates over a user type,
  not only the numeric tower.
- A polymorphic body may call a polymorphic word carrying a `Bound::User` on a forwarded
  variable, through lowering, without ICE — the `calls.rs:737` gap closed.
- The numeric tower satisfies `'T: Ord` through ordinary `impl:` blocks in `core`, with
  no per-width `impl:` written by the user.
- `Bound::Ord` no longer exists in the `Bound` enum, and `is_ord` no longer exists.
- The generic `lt` still does not swallow a user's own concrete `Vec2 lt` where `Vec2`
  has no `impl: Ord` — slice 10c's coexistence preserved through the rewritten
  admission filter.
- Every existing `'T: Copy Ord` numeric program still compiles and produces the same
  results. Codegen is *expected* to regress (a call frame per comparison); behaviour is
  not.

## Sizing

Phase shape, roughly: (1) close the cross-call lowering gap — resolve a forwarded
`Bound::User` obligation to a concrete symbol per instantiation, removing the blanket
`poly_cross_signature_supported` rejection, with the probed forwarding program as its
golden; (2) seed `Ord` as a real member-bearing trait with `Ordering` and the numeric
`impl:` blocks in `core`, deleting `Bound::Ord`; (3) rewrite `lib/cmp.sth`'s six
comparisons over `cmp`, non-inline; (4) rewrite the two overload-admission sites as
registry lookups, with a `Vec2 lt` coexistence golden; (5) diagnostics. Phase 1 is the
one with real unknowns and should be probed before the rest are sized.

## Ready to spec: yes

The design is decided and the blocker is probe-confirmed rather than assumed. Re-verify
every line citation against live `main` before locking phases — `poly.rs`,
`declarations.rs` and `ast.rs` are actively churned by other in-flight slices, and the
numbers in this brief have already drifted once.
