## Problem

`Bound` had three variants (`Copy`, `Ord`, `User(TraitId)`). `Copy`/`Ord` were reserved,
member-less trait-table entries in `RESERVED_TRAIT_MODULE`, and `is_ord` was exactly
`ty.is_numeric()`, so `'T: Ord` categorically excluded a struct or enum and a user could not
opt in: `impl: Ord for Point` and `trait: Ord 'T` were both (correctly) rejected. No live
footgun, a missing capability. `examples/traits.sth` worked around it with a separate `Order`
trait.

Two compiler gaps blocked the fix: a trait re-exported through a hub module was invisible to
a consumer importing the hub (`find_trait_in_module` was a raw one-hop table), and a
polymorphic body calling a polymorphic word carrying a `Bound::User` was rejected outright by
`poly_cross_signature_supported` (and, with the rejection removed, composed a `CallInst` with
empty `trait_calls`, so lowering could find no symbol).

## What shipped

`Ord` is an ordinary library trait in `core::cmp` (`lib/cmp.sth`):

```sooth
type: Ordering | Less | Equal | Greater ;
trait: Ord 'T  cmp ( 'T 'T -- Ordering ) ;
```

Twelve `impl: Ord` blocks (eight fixed-width ints, `usize`, `isize`, `f32`, `f64`) built from
the raw `ult`/`ugt`/`ueq` intrinsics; the six surface comparisons derived from `cmp`,
non-inline; `Ord Ordering Less Equal Greater` exported from `lib/cmp.sth` and re-exported
through `core::prelude`. `Bound::Ord`, `is_ord` and `poly_ord_bound_error` no longer exist.

Phase order as landed (spec phase numbers; commit labels are offset by one):

- **Phase 0** (`eac1b3c`, `d9e3bfa`, `6e4f8d5`) -- R1: `resolve_trait_export_origins` /
  `walk_trait_export_origin` in `src/driver.rs`, run between the trait pre-pass and the
  per-module body loop (`type_origin` is built too early to sit beside). `find_trait_in_module`
  takes `trait_origin` and falls back to it in **both** import branches (selective and bare
  qualifier), benefiting both callers (`parse_impl_decl`, `bound_trait_id`). The table is
  advisory and tolerant like the type walker: an unplaceable name is left out and the one-hop
  lookup reports the real diagnostic.
- **Phase 1** (`4b8f576`, `42031f1`, `e09764e`) -- R2/R3: the `Bound::User` blanket rejection
  in `poly_cross_signature_supported` deleted; `discover_transitive_instantiations` takes
  `word_symbols` + `trait_obligations` and builds its own `TraitResolveCtx` internally (a
  caller-side context would freeze `module`); `CrossGround` carries `tr`; `compose` runs
  `check_poly_call`'s `resolve_user_bound` loop over the grounded `subst` and stores the result
  where `trait_calls: HashMap::new()` used to be. `src/ir/func_builder/calls.rs` unchanged, as
  designed. The `(Image::Concrete(_), Bound::User(_)) => None` arm kept its code: `compose`
  grounds every mapping entry uniformly, so R2's loop already re-derives that obligation with
  the same diagnostic and span. Only the arm's comment changed.
- **Phase 2** (`65f7f45`, `ff4e170`, `18d4a86`) -- the flip: `lib/cmp.sth` + `lib/prelude.sth`;
  `Bound::Ord`/`is_ord` and every dependent arm deleted; R6's two overload-admission sites
  (`poly_admits`, `poly_sig_could_match`) rewritten as `impl: Ord` registry lookups with the
  registry threaded in and no shared abstraction introduced; R7's fixture migration;
  `unknown_capability_error` reworded to "a bound names `Copy` or a trait in scope". The
  reserved-predicate `impl:` guard and the reserved-name collision check needed **no source
  change** -- both key on what is actually seeded, so they stopped firing for `Ord` and kept
  firing for `Copy`; only their `Ord`-specific tests were deleted.
- **Phase 3** (`71c7791`, `1cce68a`, `23789ac`) -- R8/R9: `Parser` gained `is_repl: bool`
  (`true` only at the three genuine REPL construction sites, since `predicate_traits()` is also
  used by four file-parsing prepass sites), and `parse_capabilities` branches on it for the
  REPL-specific unknown-capability text; the oracle skeleton
  (`tests/phase7_slice3s_oracle.rs`) builds `examples/poly_if.sth` twice and diffs stdout plus
  per-entry-point dispatch targets, against itself until S3o flips `inline` back.

### Design rulings, as landed

- **R4, `cmp` is by value.** `&i64` is unobtainable from a scalar local, so a borrowed `cmp`
  could not be called on a numeric type from an ordinary generic body. Every `Ord`-adjacent
  word already carries `'T: Copy`, which covers element reuse.
- **R5, comparisons ship non-inline.** An `inline` word may declare no `Bound::User` variable
  (`reject_user_bound_on_combinator`), and `Ord` is now one. Measured cost: **+86.6%** on a
  comparison-heavy loop (28.9ms → 53.9ms), a real ~2x tax accepted for one slice, in exchange
  for handing S3o a differential oracle.
- **R5, NaN.** IEEE-754's unordered case has no `Ordering` variant. The float `impl: Ord`
  checks `ueq` first, then `ult`/`ugt`, and defaults a NaN pair to `Greater`; `gt`/`gte`
  compare with operands swapped (`a > b` iff `b < a`) rather than reading that arm directly.
  All six comparisons stay IEEE-correct for NaN, preserving Phase 0's D4 (NaN detected via
  `x = x`), with no fourth variant or `PartialOrd` split.
- **R6, a new same-module overlap is correct behaviour.** A module declaring both
  `impl: Ord for Vec2` and its own concrete `lt ( Vec2 Vec2 -- Bool )` now has two genuinely
  admissible candidates; `generic_concrete_overlap_error` is the right diagnostic. A carve-out
  would need real overload resolution, which this language deliberately does not have.
- **R7, `inline` migration is not mechanical.** `examples/poly_if.sth`'s `mymax`/`mymax3`
  dropped `inline` (they now forward `'T: Copy Ord` to the non-inline library `gt`, exercising
  R2's path). The two Rust fixtures that exist to pin *inline* splicing kept `inline` and
  dropped `Ord` instead, replacing `gt` with `ugt [ True ] [ False ] branch` (bare `ugt` fails:
  `if` needs `Bool`). `src/check/word_entry.rs`'s `EQ` witness dropped its incidental `Ord`.
- **R8, the REPL regression is ruled, not fixed.** See residuals.
- **R10, out of scope:** a borrowed `impl: Ord for &Point` (no autoref, so a by-value `Ord`
  excludes linear elements; `examples/experiments/binary_search.sth` stays uncompilable),
  P7.S3o, a REPL trait registry, explicit call-site instantiation (P7.S3t).

## Exit criteria

| # | Criterion | Phase | Evidence |
| --- | --- | --- | --- |
| 1 | `Ord` bounds a struct or enum, satisfied nominally by `impl: Ord for Point` | 2 | run golden |
| 2 | A polymorphic body may call a polymorphic word carrying a `Bound::User` on a forwarded variable, through lowering, without ICE | 1 | run golden + `trait_calls` assertion (`show;Show;0;Point`) |
| 3 | A **reachable** generic word's concrete-image cross-call with no matching `impl:` is a located checker error, via R2's `compose` loop | 1 | error golden, mutation-tested |
| 4 | The numeric tower satisfies `'T: Ord` through ordinary `impl:` blocks in `core` | 2 | `lib/cmp.sth` + corpus |
| 5 | `Bound::Ord` and `is_ord` no longer exist | 2 | `grep -rn "Bound::Ord\|fn is_ord" src/` empty |
| 6 | The generic `lt` still does not swallow a user's concrete `Vec2 lt` | 2 | coexistence golden, mutation-tested at both R6 sites |
| 7 | Every existing `'T: Copy Ord` **file** program compiles with no new import line | 0 + 2 | corpus builds |
| 8 | Every existing `'T: Copy Ord` program produces the same results | 2 | `tests/corpus_stdout/*.txt` byte-identical |
| 9 | The reserved-predicate `impl:`/collision guards still fire for `Copy` | 2 | surviving `Copy` assertions |

Criterion 3 is conditional on reachability; criterion 7 is scoped to file programs (the REPL is
a ruled regression). Codegen churn is expected: criterion 8 covers behaviour, not IL.

**Fixture classification pass (completed).** The raw `Copy Ord` grep found 65 lines, but 2 were
an unrelated user `Order` trait and 28 were prose: 35 real sites, of which **4** needed
non-mechanical treatment (the `EQ` witness, the two `mymax` fixtures, and
`parse_capabilities_still_folds_copy_ord_byte_for_byte`, whose assertion could no longer
construct `Bound::Ord`). A **fifth**, invisible to that grep, was `tests/phase7_slice3e.rs`'s
`sort3`, which declared its own `Ordering | Less | Equal | Greater` and was renamed to
`Rank | Under | Same | Over`; its necessity was verified by reverting it, not assumed.

## Residual gaps shipped, named

- **R3's dead-code gap.** A poly word never instantiated anywhere has its cross-call recorded
  but never composed, so an unsatisfiable `Bound::User` inside it is never checked. Precedented
  by the sibling `(Image::Concrete(ty), Bound::Copy) if !type_is_registered(...)` arm. No
  runtime unsoundness: unreached code is never monomorphized.
- **The REPL loses every comparison, not just a session's own `'T: Ord` declaration.**
  `splice_import` binds an imported word only if `w.poly.is_none()`, and retains it only if
  `is_combinator(w)`. R5 creates the first *non-inline* polymorphic library words, which fall
  into neither case, so importing and calling `eq`/`lt`/`gt`/`lte`/`gte`/`ne` is
  `error: unknown word`. Closing it needs the REPL to monomorphize a non-inline generic word
  for the first time: a separate slice. Ten tests left `#[ignore]`d with this note across
  `tests/phase1.rs`, `phase3_strings.rs`, `phase4_combinators.rs`,
  `phase4_slice10c_tail_splice.rs`. The declaration half is diagnosed:
  `` error: unknown capability `Ord` at line L, col C (`Ord` is a core::cmp trait; the REPL carries no trait or impl: registry to resolve it against -- define a word needing it in a file and load that instead) ``,
  fired only for `Ord` and only when the session declares no type of that name.
- **A generic cross-call inside a spliced combinator's own body is invisible to lowering.**
  `lib/combinators.sth`'s `times-helper` calls `lt` internally; combinators record no
  `PolyCrossCall` (P7.S3e's documented R9 scope cut), so any non-inline generic body using
  `times`/`each`/`map`/`fold`/`filter` panics at `checked user word exists`. Pre-existing, newly
  exposed by `lt` becoming a real call. Not a corpus regression (every shipped example calls
  comparisons from a body's top level or an `if` arm). Three `tests/phase7_slice3b_follow.rs`
  goldens `#[ignore]`d with the finding cited.
- **The flip reserves `Less`, `Equal` and `Greater` as variant names program-wide.** A variant
  constructor's env key is the bare surface name with no module in it, so a user enum reusing
  any one of the three captures the constructor `lib/cmp.sth`'s `impl: Ord` bodies use and the
  build fails inside `cmp`. The module-blind key is pre-existing (verified at the parent commit
  with two user modules); what changed is that one colliding party now sits in the prelude.
  Deliberately not pinned by a test: the current behaviour is the bug, not the contract.
- **`cmp` itself is not antisymmetric over floats.** `cmp(a,b)` and `cmp(b,a)` are both
  `Greater` for a NaN pair. `cmp` is not exported, but `Ord` is, so a user writing their own
  `'T: Copy Ord` word dispatches it directly. A `PartialOrd`/`Ordering?`-shaped split is the
  follow-on.
- **The +86.6% comparison tax** stands until P7.S3o. If S3o stalls again this becomes a
  standing cost worth re-litigating.
