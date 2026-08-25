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

**A second, more consequential migration cost, found by probe: `Ord` loses its
"visible with no import" property, and nothing in the compiler can give that back for
free.** `find_trait_in_module` (`src/parser.rs:508-538`) resolves a bound name in exactly
this order: own module -> the hardcoded `RESERVED_TRAIT_MODULE` entries (`Copy`/`Ord`,
today) -> an ordinary qualified/selective import. Once `Ord` is deleted from the reserved
set, it falls all the way through to ordinary import machinery, confirmed live with a
two-module probe (`'T: MyOrd` used unimported):

```
error: unknown capability `MyOrd` at line 4, col 21 (a bound names `Copy`, `Ord`, or a trait in scope)
```

-- the error message itself names `Copy`/`Ord` as the only two exemptions, confirming
there is no third mechanism to fall back on. **There used to be a general auto-visible-everywhere
mechanism (a compiler-injected prelude) and it was deliberately deleted in P8.S2** --
`core::prelude` today requires an ordinary explicit `import: core::prelude * ;` like any
other module.

**Second-order finding, also probed: re-exporting `Ord` through `core::prelude` (the
obvious fix) doesn't work either, for an unrelated reason -- and this one is a real,
previously unnamed compiler gap, now fixed and validated (see "Phase 0" below).**
`find_trait_in_module`'s selective-import lookup (`src/parser.rs:530-534`, prior to the
fix) requires the found trait's `TraitDecl.module` to equal the *immediately* imported
module -- a raw one-hop table. A trait declared in module A and merely re-exported
(`export: Name ;`, not declared) by hub module B is invisible to a consumer C importing
only B, even with an explicit `import: B | Name | ;`, confirmed live:
```
error: unknown capability `MyOrd` at line 3, col 14 (a bound names `Copy`, `Ord`, or a trait in scope)
```
Struct/enum type names already have a purpose-built multi-hop walker for exactly this
(`resolve_type_export_origins`/`walk_type_export_origin`, `driver.rs:352-421`) and
intrinsics have their own separate one (P7.S3q). Traits had neither -- nobody wrote the
trait equivalent. **Built, tested, and left as a validated, ready-to-land fix**
(`resolve_trait_export_origins`/`walk_trait_export_origin`, mirroring the type walker
exactly): full diff is five files (`driver.rs` +84, `parser.rs` +69/-6 threading a
`trait_origin` parameter through every `Parser`/`parse_bodies` call site, two trivial
test-fixture updates), two new end-to-end goldens (one exercising the selective-import
form, one the bare-qualifier form `h::Greet`, both actually **run** and asserting real
output, not just compiling), full `cargo test --no-fail-fast` green (no regressions),
`cargo fmt --check` and `cargo clippy -- -D warnings` clean. The original one-hop and
`Copy`/`Ord` reserved-predicate lookups are unchanged and re-verified working. This must
land as **this slice's own phase 0** (see Sizing) -- it is a real, independent compiler
fix, not a design choice, and nothing about `Ord` proper can be built on top of a broken
re-export path.

With phase 0 landed, the import-visibility question above resolves cleanly rather than
forcing a migration: declare `Ord` in `core::cmp` (alongside the rest of the comparison
machinery, where it belongs) and re-export it through `core::prelude`
(`export: Ord ;`). Every existing `'T: ... Ord` call site checked already imports
`core::prelude *` (`examples/poly_if.sth:6`: `import: core::prelude * ;    \ Bool comes
through here, the prelude's own re-export.` -- the exact same pattern this slice now
needs for `Ord`), so the hub re-export reaches all of them for free, with **zero new
import lines** -- the exit criterion below is met exactly, not approximately. Counted
live for reference: 14 real `'T: ... Ord`-bound signatures exist today across
`lib/cmp.sth` (6, own-module, never needed an import anyway), `docs/roadmap/P8/dogfood/core/cmp.sth`
(6, itself stale, see below), and `examples/poly_if.sth` (2, covered by the existing
`core::prelude *` import once phase 0 and the hub re-export are both in place).

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
  raw comparison intrinsics (`ueq`/`ult`/.../`une`) and `bool`'s `branch`.
  `docs/roadmap/P8/dogfood/core/cmp.sth` is **not** a live planning artifact for this --
  checked by probe and confirmed stale on two independent axes: it predates the
  `bool`->`Bool`/`true`->`True` rename (`b2c0d9a`, 2026-08-22; the dogfood file still uses
  lowercase and no longer parses standalone), and it predates P8.S2's real package split
  (`d539032`, 2026-08-21) that it was ostensibly sketching -- `docs/roadmap/P8/dogfood/README.md`
  says outright "none of this compiles." Ignore it; `lib/cmp.sth` is the only real file.
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
  cmp ( 'T 'T -- Ordering )
;
```

`impl: Ord for i64` and one per remaining numeric width are written in `core`, each body
built from the raw comparison intrinsics (`ult`/`ueq`/...) exactly as `lib/cmp.sth`'s
bodies are today. The six surface comparisons (`lt`/`gt`/`eq`/`lte`/`gte`/`ne`) are then
derived from `cmp` rather than from the intrinsics directly, and `Bound::Ord` is deleted
from the `Bound` enum.

**Correction from probe: `cmp ( 'T 'T -- Ordering )`, by value, not `&'T &'T`.** The
previous draft of this brief specified borrows, reasoning that a by-value `cmp` would
stop `sort` reusing an element after comparing it. Probed and falsified twice over: (1)
`&i64` is not obtainable from a plain scalar local at all --
`cannot borrow the scalar local 'a' of type 'i64' ... a scalar has no address; borrow a
field or an aggregate instead` (`word_families.rs:1397`, confirmed live) -- so a borrowed
`cmp` cannot be called on `i64` from an ordinary generic body without routing every
numeric comparison through a synthetic one-field wrapper struct, which is not a real
option; (2) it is also unnecessary -- every existing `Ord`-adjacent generic word already
carries `'T: Copy` alongside `'T: Ord` (`lib/cmp.sth`, `mymax`, `arrays.sth`'s
`sort`/`bin_search`'s own quotation comparator, which is already by-value:
`~[ 'T 'T -- i64 ]`), so element reuse after a comparison is `Copy`'s job, not `cmp`'s.
A by-value `cmp` matches the shape the rest of the language already uses and sidesteps
the scalar-borrow wall entirely.

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

**Revised down from "structurally harder" to a small, checker-only fix -- built and
verified, not just characterized.** An initial trace judged this new plumbing on both
the checker and lowering side. A follow-up probe built it instead: `calls.rs:737` needed
no change at all. `CallInst.trait_calls` (a `HashMap<Span, String>`) already flows from
the checker to lowering unconditionally, for the *ordinary* concrete-caller path
(`resolve_user_bound` populates it once the caller's own `Subst` is ground). The
cross-call compose step (`CrossGround::compose`, `poly.rs`) was simply building its
`CallInst` with `trait_calls: HashMap::new()` -- hardcoded empty, since a `Bound::User`
never reached this point before. The fix: thread a `TraitResolveCtx` into `CrossGround`
(built fresh inside `discover_transitive_instantiations` from `module.traits`/`.impls`,
since a caller-side context would freeze `module` while this function still needs `&mut
module`), and once `compose` grounds the callee's `θ` (which it already does, to resolve
ordinary output types), call `resolve_user_bound` there too, exactly as the concrete path
does, and store the result in the `CallInst` it already produces. Lowering picks it up
with zero changes, because it was already reading `trait_calls` by span for every
`CallInst` regardless of which path produced it.

Verified in a probe worktree: `cargo build`, `cargo test --no-fail-fast` (full suite),
`cargo fmt --check`, and `cargo clippy -- -D warnings` all green, plus a new passing
test (`check_generic_cross_call_discharges_a_forwarded_user_bound`) asserting the
composed `CallInst.trait_calls` actually resolves to `impl: Show for Point`'s symbol,
replacing an existing test's stale case that had pinned the old blanket rejection. Diff
is five call sites: `poly_cross_signature_supported`'s rejection deleted,
`discover_transitive_instantiations` gains two parameters
(`word_symbols`, `trait_obligations`) to build its own `TraitResolveCtx`, `CrossGround`
gains a `tr` field, and `compose` gains the same `resolve_user_bound` loop
`check_poly_call` already runs. The probe's own worktree was reverted to pristine per
instructions (this is a validated design, not a landed change) -- **re-derive rather than
copy-paste at implementation time**, and re-run the exact test added here as this phase's
golden.

**The concrete-image soundness hole (below) was exercised in the same probe session but
its exact output was not recoverable from the archived session log** -- the probe's own
closing summary says the edge case "errors correctly," but the transcript format used for
this run doesn't retain tool stdout, so that claim rests on the probe's word alone, not on
pasted evidence this brief can verify. Treat it as unconfirmed and re-run explicitly
before relying on it.

**A second, independent soundness hole, found by the same probe.** `(Image::Concrete(_),
Bound::User(_)) => None` (`poly.rs:1886`) is commented "unreachable", gated only by the
same blanket rejection. With that rejection removed, a generic word can call a
`Bound::User`-bounded generic callee on a *concrete* type carrying **zero** matching
`impl:`, and the program passes `check::check` in full silence — it dies later only on an
unrelated pre-existing lowering panic (`driver.rs:757`, an unrelated `Ref`-shape lookup),
not on any diagnostic naming the missing impl. `None` here means "satisfied"; for a type
with no impl at all, that is a wrongly-typed program silently accepted. **This arm must
be fixed in the same phase as the cross-call gap**, not deferred: closing the cross-call
gap without it converts a currently-inert bug into a live one, since the unrelated
lowering panic that happens to mask it today is not a mechanism to depend on.

### Scope: the comparisons drop `inline`, and that is deliberate

`is_combinator(word)` is exactly `word.declares_inline` (`combinators.rs:155-157`) —
a quotation parameter is irrelevant. All six comparisons in `lib/cmp.sth` are `inline`
today, so a `'T: Ord`-bounded `lt` would hit `reject_user_bound_on_combinator` on its own
declaration. That rejection is **P7.S3o**, parked with two designs found unsound in
review, whose failure mode is silent dispatch to the wrong `impl:`.

This slice therefore ships the six comparisons **non-inline**. Each becomes a real
monomorphized call frame, and QBE performs no cross-function inlining, so a comparison
that is one `ult` today becomes a leaf call. **Measured, not assumed:** a 20M-iteration
comparison-heavy loop (two comparisons per iteration, identical output both ways,
`19958023`) runs in 28.9ms mean with today's inline `lt`/`gt` (7 runs, stdev 1.2ms) versus
53.9ms with a hand-written non-inline equivalent (7 runs, stdev 1.8ms) -- **+86.6%**,
spreads non-overlapping. Disassembly confirms the mechanism: straight-line
`cmp`/`setl`/`cmovcc` inline, versus a real `call` per comparison (40M calls total) with a
full prologue/epilogue each, non-inline. This is a real, substantial, roughly 2x tax on
comparison-heavy code -- not noise, and not something to wave through lightly. Accepted
for one slice's duration on the following reasoning:

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
swallowing a user's own concrete `Vec2 lt`. **Both confirmed load-bearing by mutation,
not assumed.** A baseline program declaring both a concrete `lt ( Vec2 Vec2 -- Bool )`
and the library's generic `lt ( 'T: Copy Ord 'T -- Bool )` in the same module compiles and
dispatches correctly today. Neutering `poly_admits` alone breaks it at *declaration*
time (the two candidates are no longer disambiguable). Restoring that and instead
neutering `poly_sig_could_match` alone breaks it at the `Vec2 Vec2 lt` *call site*: the
generic candidate now wins selection, gets instantiated, and its body's `ult` rejects the
non-numeric `Vec2` operands with a real type error. Both are real, loud rejections —
neither mutation produced a silent mis-dispatch — but both sites are independently
necessary for this coexistence and both go dead the moment `Bound::Ord` is deleted. Both
must become a real `(TraitId, Type)` registry lookup: decline unless the operand type has
an `impl: Ord`.

### Diagnostics

`poly_ord_bound_error` and `poly.rs:2285`'s `Bound::Ord => "Ord"` naming disappear with
the variant. An unsatisfied `Ord` becomes an ordinary user-trait failure, and should name
the missing `impl:` the way a `Bound::User` failure does.

### Named, deliberately out of scope: a borrowed impl for a linear element

The registry key is `(TraitId, Type)`, and `Type::Ref` is a distinct `Type` from its
referent -- `impl: Ord for &Point` is not a variant of `impl: Ord for Point`, it is an
independent entry. Probed live: it parses (`parse_type_expr` already handles `&` in an
impl target position), passes the orphan rule (a `Type::Ref` target falls to
`impl_target_module`'s scalar-like `_ => None` arm, same bucket as `i64`), and a generic
word bounding its own `'T` directly as the reference type (`is_less ( 'T: Ord 'T -- Bool
)`, called as `&p1 &p2 is_less`) dispatches and runs correctly.

This does **not** subsume the by-value `cmp` above, and is not a variant reading of the
same trait -- confirmed by probe, not assumed: a generic word whose `'T` infers to the
*owned* type (`3 4 is_less`, `'T` = `i64`) fails outright against a registry that only
holds `(Ord, &i64)` --

```
error: cannot instantiate `'T` of `is_less` with `i64`
  `i64` does not satisfy `Ord`: no `( i64 i64 -- Ordering )` found
```

-- there is no autoref: the checker never tries "does `&i64` satisfy this, given an
`i64`". A bare literal or an unaddressable local can never produce the `&i64` this path
would require, so an owned-type impl remains mandatory for ordinary numeric code to work
at all; a ref-type impl is an *additional*, separate registration, not a replacement.

The reason to name it here rather than drop it: `bin_search_internal`
(`examples/experiments/binary_search.sth`, aspirational syntax, does not compile as-is)
already assumes exactly this shape -- `Slice['T: Ord 'N] &'T`, comparing a borrowed
element, with no `Copy` bound anywhere. That is deliberate, not an oversight: `Copy`
alongside `Ord` is only there so a by-value `cmp` doesn't strand the element it just
consumed, and a **linear** element -- the language's own default, an owned value used
exactly once -- can never carry `Copy`. A by-value `Ord` therefore categorically excludes
linear elements from ever being sorted or searched; only a borrowed `cmp` can compare a
linear element without consuming it. `'T` does not "support both" through one impl --
there is no receiver polymorphism, no auto-deref search from `'T` to `&'T` -- so a
linear-friendly `sort`/`bin_search` needs its own borrowed-`cmp` impls and, most likely,
its own generic-word bodies written against a `&'T`-shaped comparison rather than the
by-value one this slice ships. Real, and worth a named follow-on once a concrete linear
consumer needs it, but it is not a gap in this slice's own promise: **the exit criteria
below only ever commit to `'T: Copy Ord`**, exactly the numeric/Copy-struct case that
exists in the codebase today.

## Dependencies / sequencing

- **Blocks nothing on P7.S3o, but hands it its entry condition.** S3o's brief parks it
  with "revisit only if a concrete program actually needs bound dispatch on a
  combinator's own type variable". This slice is that program. S3o becomes the named
  follow-on: re-`inline` the six comparisons, with the differential harness above as its
  entry condition and its own benchmark to justify itself against.
- **No dependency on P7.S3p.** `cmp`'s receiver is trailing; the non-trailing fix is not
  needed.
- **No dependency on P7.S3k**, already closed.
- **Correction: P8.S2 already shipped (`d539032`, 2026-08-21) -- this brief's own
  "sequence before" language was stale the moment it was written.** Checked against
  `slice2-brief.md`/`slice2-spec.md` directly: P8.S2 deleted the compiler-injected
  prelude, gated `intrinsics` imports, and split `lib/` into a real `core` package --
  import/visibility mechanics only. It never touched `Ord`'s satisfaction mechanism, and
  both docs mention `'T: Copy Ord` only in passing as an example of an *existing* bound
  used unchanged. There is no live sequencing hazard: nothing P8.S2 shipped assumes `Ord`
  is a predicate rather than a trait. Retained here only as a correction, not a
  dependency -- the real, still-open dependency this slice creates is the import-visibility
  question above ("a second, more consequential migration cost"), which is orthogonal to
  P8.S2 and unresolved by it.

## Exit criteria

- `Ord` bounds a struct or enum, satisfied nominally by an `impl: Ord for Point` block —
  a comparison-bounded generic word (`sort`, `bin_search`) instantiates over a user type,
  not only the numeric tower.
- A polymorphic body may call a polymorphic word carrying a `Bound::User` on a forwarded
  variable, through lowering, without ICE — the `calls.rs:737` gap closed.
- A polymorphic body calling a `Bound::User`-bounded generic word on a *concrete* type
  with no matching `impl:` is a located checker error, not a silent accept — the
  `(Image::Concrete(_), Bound::User(_)) => None` soundness hole (`poly.rs:1886`) closed
  in the same phase as the cross-call gap, not deferred.
- The numeric tower satisfies `'T: Ord` through ordinary `impl:` blocks in `core`, with
  no per-width `impl:` written by the user.
- `Bound::Ord` no longer exists in the `Bound` enum, and `is_ord` no longer exists.
- The generic `lt` still does not swallow a user's own concrete `Vec2 lt` where `Vec2`
  has no `impl: Ord` — slice 10c's coexistence preserved through the rewritten
  admission filter.
- Every existing `'T: Copy Ord` numeric program still compiles **with no new import
  line** -- delivered via phase 0 (trait-through-hub fix, validated) plus declaring `Ord`
  in `core::cmp` and re-exporting it through `core::prelude`, which every checked call
  site already imports.
- Every existing `'T: Copy Ord` numeric program still produces the same results. Codegen
  is *expected* to regress (a call frame per comparison); behaviour is not.

## Sizing

Phase shape: **(0) land the trait-through-hub fix -- built, tested, and verified
clean (full suite/fmt/clippy) in a probe worktree, diff already exists** (five files:
`driver.rs` gains `resolve_trait_export_origins`/`walk_trait_export_origin` mirroring the
existing type-name walker; `parser.rs` threads a `trait_origin` table through every
`Parser`/`parse_bodies` call site; two new end-to-end goldens, both run and checked for
real output, not just compiled). Re-derive against current `main` rather than
copy-paste (the probe worktree is scratch); this is a real, independent compiler fix with
no open design question, land it as its own reviewable phase before anything `Ord`-shaped
touches it; (1) close the cross-call lowering gap and the concrete-image soundness hole
together -- **also a validated design, not just a characterization**: thread a
`TraitResolveCtx` into `CrossGround`, resolve the callee's `Bound::User` obligations via
`resolve_user_bound` once `compose` grounds its `θ`, and store the result in the
`CallInst` it already produces -- `calls.rs` needs no change, since it already reads
`trait_calls` by span regardless of which path populated it. Built and verified clean in
a separate probe worktree; re-derive rather than copy-paste, and reuse the probe's own
golden shape (`check_generic_cross_call_discharges_a_forwarded_user_bound`). The
concrete-image half (`(Image::Concrete(_), Bound::User(_)) => None`) was exercised in the
same probe but its output wasn't recoverable from the session log -- re-confirm this
specific case fresh, don't inherit it as closed; (2) declare `Ord` in `core::cmp`, seed it
as a real member-bearing trait with `Ordering` and `cmp ( 'T 'T -- Ordering )` (by value,
per the probe correction above), plus the numeric `impl:` blocks, deleting `Bound::Ord`,
and re-export it through `core::prelude`; (3) rewrite `lib/cmp.sth`'s six comparisons
over `cmp`, non-inline; (4) rewrite the two overload-admission sites as registry lookups,
with the probed `Vec2 lt` coexistence program as its golden; (5) diagnostics.

## Ready to spec: yes

The design is decided, both blocking gaps (cross-call lowering, trait-through-hub
re-export) have validated, tested fix designs rather than characterizations, and the
import-visibility question is closed outright -- declare `Ord` in `core::cmp`, re-export
through `core::prelude`, zero new import lines anywhere. Re-verify every line citation and
re-derive both probed diffs against live `main` before locking phases (do not copy-paste
from the scratch worktrees) -- `poly.rs`, `declarations.rs`, `parser.rs`, `driver.rs` and
`ast.rs` are actively churned by other in-flight slices, and the numbers in this brief
have already drifted once.
