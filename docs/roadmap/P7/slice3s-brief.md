# P7.S3s brief — `Ord` as a library trait, not a compiler-hardcoded bound

## Problem, confirmed live against current `main`

`Bound` (`src/ast.rs:1417-1421`) has three variants: `Copy`, `Ord`, and `User(TraitId)`.
The first two are **reserved, member-less trait-table entries**
(`seed_predicate_traits`, `ast.rs:1528-1546`) that exist only so `parse_capabilities`
(the `'T: Copy Ord` parser) can look every bound name up through one uniform mechanism
(`ast.rs:1520-1527` comment). Satisfaction is never nominal for either: `poly_is_copy`
checks a closed set of concrete-type predicates (`is_copy`), and `is_ord`
(`src/check/poly.rs:120-122`) is exactly:

```rust
pub(super) fn is_ord(ty: Type) -> bool {
    ty.is_numeric()
}
```

— a hardcoded numeric-tower check, consulted at every `Bound::Ord` discharge site
(`declarations.rs:1348`, `poly.rs:1211`, `poly.rs:3708`, `poly.rs:3910`). None of these
sites ever consult the `impl:` registry `Bound::User` dispatch already uses
(`check_impl_decls`, `declarations.rs:407-482`; whole-program `(TraitId, Type)` lookup,
`slice3e-spec.md` design ruling 9). **This produces a live, confirmed footgun**: writing
`trait: Ord ...` is rejected as a name collision with the reserved predicate entry
(`trait_name_collision_error`, `declarations.rs:333-336`, `colliding_name_kind` — not yet
traced whether it fires here or elsewhere, needs a probe), but nothing stops `impl: Ord
for Point` if it *were* somehow reachable, since a `Predicate`-kind `TraitDecl` carries
`members: Vec::new()` (`ast.rs:1531-1544`) — zero required members means
`check_impl_decls`'s missing-member loop (`declarations.rs:472-480`) is vacuously
satisfied, and the impl would type-check while doing nothing: `Bound::Ord`'s discharge
sites never look at the impl registry, only at `is_ord`'s numeric check. **Not yet
probed** whether the name collision actually blocks this path today or whether it is
live and silently inert — first thing the spec phase must confirm.

The consequence for a user: `'T: Ord` bounds a struct or enum out categorically. `sort`,
`bin_search`, or any comparison-bounded generic word can only ever be instantiated over
the numeric tower, by construction, no matter how many `impl:` blocks a user writes for
their own type. This is the gap `Program 2` of `docs/roadmap/P7/slice3-dogfood.md`
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
  &'T -- Ordering )` (`examples/traits.sth`) has its bound variable as the *last*
  declared input (`&'T`), so `member_ends_in_trait_var` (`declarations.rs:361-364`)
  accepts it and `receiver_ty_var` dispatches it correctly today — **this slice does not
  depend on P7.S3p** (the non-trailing-receiver gap) for a `cmp`-shaped `Ord`. It would
  only become relevant if `Ord`'s member set grows a shape like `clamp ( &'T lo:'T hi:'T
  -- 'T )` with the receiver non-trailing, which is not required to satisfy the roadmap's
  own `Ord` semantics.
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

## The actual redesign: what has to move, and what it breaks

`Bound::Ord` is not a narrow constant to swap — it is threaded through the checker as a
first-class enum variant, distinct from `Bound::User(TraitId)`, at every site listed
above (four discharge sites plus the `Bound` enum itself, plus every error message
naming `Ord` by variant match rather than by trait name lookup, e.g.
`poly_ord_bound_error`). Turning `Ord` into a real trait means one of two shapes, neither
yet designed:

1. **Collapse `Bound::Ord` into `Bound::User(TraitId)`, with `Ord`'s `TraitId` seeded to
   a real, member-bearing `trait: Ord 'T ... ;`** (mirroring `Order` in
   `examples/traits.sth`, but under the reserved name), with `impl: Ord for i64` / `for
   f64` / ... written once for the numeric tower in a core library module, replacing
   `is_ord`'s hardcoded predicate with real dispatch through the `(TraitId, Type)`
   registry. This deletes the `Bound::Ord` variant entirely and every one of its four
   discharge sites becomes an ordinary `Bound::User` discharge — but this changes the
   *performance/mechanism* of every existing numeric comparison: today `is_ord` is a
   zero-cost `ty.is_numeric()` match; after this move, satisfying `'T: Ord` for `i64`
   means resolving through the whole-program impl registry every time, which changes
   both how `check_poly_call`'s bound-check reports failure (today's `poly_ord_bound_error`
   names "Ord" as a fixed capability; after, it would need to look and read exactly like
   any other unsatisfied user trait) and how many `impl:` blocks the core library needs
   to seed (one per numeric width `i8..i64`, `u8..usize/isize`, `f32`/`f64` — a dozen-plus
   boilerplate impls, unless the impl mechanism grows some form of blanket/derived impl,
   which does not exist and is out of scope to invent here).

2. **Keep `Bound::Ord` as the fast numeric-tower path, and add a second,
   separately-named library trait (`Order`, matching the dogfood example) that a struct
   or enum satisfies nominally**, leaving `'T: Ord` exactly as restrictive as it is today.
   This is strictly additive and touches none of the four existing discharge sites, but
   it does not answer the user's actual question — `Ord` stays a numeric-only intrinsic,
   and a struct-bearing generic word has to spell a different bound name than the one
   the language ships for numeric comparisons, which is the asymmetry motivating this
   slice in the first place.

**Not yet recon'd or decided which of these is right** — this is the central open design
question for the spec phase, not a mechanical migration. (1) is the literal reading of
"make `Ord` a library trait" but has a real cost (loses the zero-cost numeric fast path,
needs a boilerplate-impl story) and a real blast radius (every one of the four
`is_ord`/`Bound::Ord` sites, plus every diagnostic naming `Ord` by variant). (2) is
smaller and lower-risk but arguably doesn't deliver what was asked — it just files a
second trait alongside the existing intrinsic rather than replacing it. A third option
worth at least naming for the spec to rule out or in: keep `Bound::Ord`'s fast numeric
path as an *implicit* satisfaction of the same `Ord` trait (i.e., `is_ord(ty)` short-
circuits the registry lookup for the numeric tower, and only a non-numeric type falls
through to a real `(TraitId, Type)` lookup) — this would preserve the zero-cost path for
the common case while still letting a struct `impl: Ord for Point` genuinely work,
without a dozen boilerplate numeric impls, but blurs "nominal satisfaction" (design
ruling 1's whole premise) with an implicit fallback, which the spec phase needs to weigh
against S3e's own stated design philosophy before choosing.

## Dependencies / sequencing

- **No dependency on P7.S3p.** `Ord`'s natural member shape (`cmp`, binary, trailing
  receiver) never needs the non-trailing-receiver fix.
- **No dependency on P7.S3k**, already closed.
- **Overlaps P8.S2's planned `lib/cmp.sth` migration.** P8.S2 is moving the *surface*
  comparison words (`eq`/`lt`/`gt`/...) out of the compiled-in prelude into a
  gated-import library module, still `'T: Copy Ord`-bounded exactly as today — it does
  not touch what `Ord` *means*, only where the comparison words live. If this slice
  changes `Bound::Ord`'s satisfaction mechanism (option 1 above), P8.S2's migration
  needs to land against the *new* `Ord`, not the old numeric-only one, or the two land in
  the wrong order and one of them has to be redone. Recommend sequencing this slice
  **before** P8.S2's `lib/cmp.sth` migration lands, or explicitly coordinating the two
  specs so P8.S2 is written against whichever `Ord` shape this slice produces.

## Exit criteria (proposed, not yet roadmap-committed)

- `Ord` bounds a struct or enum, satisfied nominally by an `impl:` block, exactly as
  `Order` does in `examples/traits.sth` today — a comparison-bounded generic word
  (`sort`, `bin_search`) can be instantiated over a user type, not only the numeric
  tower.
- The numeric tower still satisfies `'T: Ord` with no per-width boilerplate `impl:`
  the user has to write (mechanism TBD per the open design question above).
- Every existing `'T: Copy Ord` numeric program (the whole of `lib/cmp.sth`,
  `examples/poly_if.sth`'s `mymax`, `lib/arrays.sth`'s `sort`/`bin_search`) keeps
  compiling and running unchanged — this slice must not be observable to code that never
  touches a non-numeric `Ord` bound.

## Sizing

Not sizeable yet — the central design question (option 1 vs 2 vs the hybrid) is unresolved
and changes the shape of every subsequent phase. Recommend the spec phase open with a
paper-traced design comparing the three options against real call sites (mirroring how
S3k's brief validated its design before locking phases), rather than sizing phases against
an undecided mechanism.

## Ready to spec: no — one design question blocks phase planning

Do not hand this to spec-writer until the collapse-vs-add-a-second-trait-vs-hybrid
question above is resolved with a paper-traced design against the four `is_ord`/
`Bound::Ord` call sites and `seed_predicate_traits`. Once that's settled, re-verify every
citation above against live `main` (`poly.rs`/`declarations.rs`/`ast.rs` are active files
other in-flight slices also touch) before locking a spec.
