# P7.S11-follow spec — Check-time monomorphs invisible to the enclosing word

> **Status: implemented.** Phase 1 (infrastructure) landed in `73eb12b`; Phase 2
> (behavior + test migration) in `c8e8aa6`. This spec is condensed to the *why*
> and *what*; for the *how* (exact sites, closure shapes, per-call-site wiring)
> read those two commits. The delivery/test mechanics below are kept only where
> they carry an enduring ruling.

## What this slice fixes

A generic type whose *only* instantiation in a program comes from a combinator's
own signature (the `map`/`and_then`/`wrap` shape P7.S11 exists to unblock) did
not build end to end. The combinator's own definition checked, but every site
that *used* the result failed with `unknown word` on the generated
constructor/accessor, and an eliminator over such a value was rejected as
"ungrounded."

Root cause: the concrete call-site `env` and the `enums`/`structs` decl slices a
word is checked against are frozen before the per-word loop. A **check-time**
monomorph, minted while checking a word's body, is flushed into the live
`GenericTypes` cell but is invisible to:

1. the constructor/accessor `env` the same word (and later words) look up
   against;
2. id-indexed decl reads (`drop`/layout/`is_copy`/tag, and the eliminator's
   `gate_decl`/`enum_decl`/`variant_type` reads), which panic or miss on an id
   past the flushed slice;
3. the eliminator's scrutinee classification, frozen at pre-loop
   `eliminator_registry` build time as `Generic` and never re-consulted after a
   later mint.

Two originating mint sites feed this gap, and both write into the *same* live
`generics_cell` regardless of call shape: a combinator's own splice-site output
grounding, and an ordinary mid-word poly call's `apply_subst` mint. Because both
land in one live cell, a single fallback *at the point of use* covers both — no
splice-scoped `env` clone, no second wiring site.

## The fix — four parts, one entangled mechanism

Implemented across `73eb12b` (Part 2) and `c8e8aa6` (Parts 1, 3, 4).

- **Part 1 — Splice-site output grounding** (`inline_combinator`). Force
  `apply_subst` over the combinator's declared `sig.outputs` before the body
  splice, so the output monomorph is minted into the live cell ahead of any use.
  **Ruling: propagate `apply_subst`'s `Err`** (an unbound declared output). Do
  not swallow it — a broken signature must not silently splice a garbage
  monomorph. Golden 5 stays green because it asserts a substring of the same
  helper's text.

- **Part 2 — Id-indexed decl fallback** (`GenericTypes::struct_decl` twin of the
  P7.S12 `enum_decl`; closure-taking `Ctx` accessors). Every id-indexed decl
  read on the **concrete checker path** falls back to the live cell's unflushed
  mint when the id is past the flushed slice. This includes the `is_copy` /
  `contains_reference` / `is_linear` reads (routed through an *extended* slice =
  flushed prefix ++ pending tail), because Parts 1/4 open a real panic hole:
  once a check-time-only monomorph's constructor resolves, an ordinary program
  can `dup`/`over` it, `fill` an array of it, or just bind it to a local — each
  reaches an unbounded `structs[id]`/`enums[id]` read. Every concrete-path call
  site must be covered, not a representative subset.
  - **Not wired: the `poly.rs` body-walk reads.** Its direct id-indexed reads
    are **pre-existing exposure at HEAD** (the pre-walk flush covers a
    *pre-walk* mint, not a *mid-walk* one), not newly reachable through this
    slice — left untouched because out of scope here, *not* because they are
    safe.
  - **Footgun: the generic *header* table** (`generics.enums[idx]` /
    `generics.structs[idx]`) is a different registry keyed by generic-decl
    index, one character apart from the mint reads. Rerouting it is a bug.
  - Dedup safety is pre-proven: `instantiate_*` dedup by memo key, so the
    fallback never returns two entries for one monomorph.

- **Part 3 — Eliminator scrutinee grounding from the live stack**
  (`scrutinee_enum_id_of_family`). When classification is the frozen `Generic`
  entry, recover the scrutinee's own concrete `Type::Enum(id, _)` from the stack
  and, if it resolves to a *real mint*, proceed with that id.
  - The scrutinee is **not** `stack.last()` — the arms are still stacked above
    it. Part 3 replicates the arm-collection scan as a **peek-only** helper
    (shared with the existing destructive scan in `check.rs`, which alone keeps
    its pops), leaving the stack intact.
  - **Ruling: do not ship permissive.** The fallback must confirm the id
    resolves to an *actually minted* decl, not merely that the stack type's tag
    matches the family — a poly call's unification can leave a concrete-looking
    `Type::Enum` on the stack without anything grounding it. A genuinely
    ungrounded call must keep getting the honest "cannot eliminate it while it is
    ungrounded" diagnostic. This actual-mint guard is **defensive coding with no
    integration witness** (no real program can present a concrete-looking but
    never-minted enum scrutinee — producing an enum value mints it); it is
    witnessed only by a hand-built-`Ctx` unit test.

- **Part 4 — Shared `env`-miss mint fallback** (`mint_fallback_candidates`). On
  an ordinary `env.get(name)` miss, re-derive the generated constructor/accessor
  sigs for the live cell's pending mints and return any matching name. This one
  fallback covers **both** originating mints (splice-local and ordinary poly
  call) because both share the live cell — gate (v) is not a second wiring site.
  - **Id-derivation runs over the *extended* slice, never the pending tail
    alone**, then skips the flushed-prefix length; over the tail alone, each
    candidate would mint at `from_index(0..)` — a wrong, colliding id.
  - **Invariant it relies on** (not automatic): the flushed prefix and the
    pending tail meet exactly at `ctx.enums()/structs().len()`, because the
    concrete-word loop and poly-body path rebase the live cell to that length
    immediately before check and flush only after. A future reader rerouting a
    derivation past this rebase/flush bracket would silently mint colliding ids.
  - **Name-collision ruling.** Variant-ctor env keys are module-blind, so two
    pending mints can share a surface name. The fallback returns *all* matches
    and dispatch stays **first-wins** (the standing env-overload behavior) — a
    drop-in for a missed `env` entry must not invent a stricter rule than a
    present entry would have had.

## Out of scope (do not widen)

- The standalone-combinator path (`ground_into_word_scoped_registries`) mints
  into a dropped clone and must not reach the live `env`. Unchanged.
- `poly_env`: a poly word's signature never depends on a concrete monomorph
  pre-existing in `env`. Unchanged.
- Pre-loop passes reading an `env` snapshot (`check_extern_decls` /
  `check_main_effect`): they run before the per-word loop over source text, which
  can only name a type that already exists. No gap.
- Refuted design: "re-run the generated-sig helpers over each word's post-flush
  tail." At the failing lookup nothing has minted yet; resolving the constructor
  is itself what triggers the mint.

## Growth-structure note

This touches six files but is **one entangled mechanism** (a check-time mint made
visible to its enclosing word), not a new subsystem, and does not trip the
split/elevate signals. It *elevates a fallback pattern already used once*
(`enum_decl` gets its `struct_decl` twin, both surfaced through `Ctx` accessors
at the lowest common ancestor of their consumers). No module split beyond the
`Ctx` accessor was warranted.

## What the tests witness (enduring rationale)

Full detail in `c8e8aa6`; the load-bearing points:

- **Four bug-pinning goldens flipped** to `build_and_run` (goldens 6, 9, 10 in
  `phase7_slice11.rs`; one fixture in `phase7_slice12.rs`). Golden 6 and the
  slice12 fixture **assert stdout**, not just exit code, so a wrong-arm or
  wrong-monomorph resolution is caught (`8\n` and `7.5\n` respectively).
- **Golden 6b** (new): the minimal, unconfounded control — enum construction +
  `drop`, no eliminator, one header, one mint — isolating Part 4's constructor
  fallback and Part 2's enum drop/layout reads from Part 3's machinery and from
  golden 9's stale-base confound.
- **`dup` and bind witnesses** (new): two *independent* panic-hole witnesses for
  the extended-slice wiring. `is_copy` (reached via `dup`/`over` in `check.rs`)
  and `is_linear` (reached via local bind in `terms.rs`) are disjoint call
  sites, so fixing one does not imply the other; each fixture panics without its
  wiring.
- **Fabricated-instantiation guard re-homed, not retired.** The negative
  assertions (`!contains("nothing in this program instantiates")`,
  `!contains("i64")`) move onto a NEW `build_error` fixture whose `main` is
  checked **before** `probe` — so a live `Pair[f64]` monomorph exists at
  diagnostic time (the per-word loop aborts on first error; `probe` first would
  mean zero live monomorphs, the weaker state the re-home avoids). It stays
  rejected because `probe`'s scrutinee is `f64`, never a `Type::Enum`.
- **Part 1's grounding is mutation-proven** against golden 6 (deleting it
  re-breaks the golden). **Part 3's actual-mint guard has no integration
  witness** and is not falsely claimed to have one — unit-test only.
- slice12's flipped fixture also carried an **orthogonal `Nil`-arm
  stack-linearity bug**, fixed in the same commit; it is not part of the
  mechanism.

## Delivery (as landed)

Two phases, because Parts 1/3/4 are the behavior change (the moment they land the
bug-pinning goldens turn red) while Part 2 is pure infrastructure (no flips):

- **Phase 1 (`73eb12b`) — id-indexed decl fallback infrastructure.** Part 2
  only: `struct_decl` twin, `Ctx` accessors, all concrete-path id-indexed reads
  wired. No behavior change, zero golden flips. `poly.rs` untouched.
- **Phase 2 (`c8e8aa6`) — check-time mint resolution + test migration.** Parts
  1, 3, 4, and the four coupled fixture migrations plus new witnesses, landing
  atomically green.

## Acceptance criteria (met)

1. Goldens 6, 9, 10 and the slice12 fixture flip to `build_and_run` / exit 0
   (golden 6 and the slice12 fixture assert stdout); golden 6b builds and runs.
   Goldens 4, 5, 7, 8 and the three slice12 rejection tests are unchanged.
2. Each new guard has beside-it unit coverage. Part 1's grounding is mutation-
   proven against golden 6; the Part-3 actual-mint check is defensive coding with
   no integration witness (and the false "deleting it re-breaks slice12" claim is
   not asserted).
3. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green, with
   exactly the four flips plus four added tests and the slice12 orthogonal fix.
4. The fabricated-instantiation guard is re-homed onto a scrutinee-mismatch
   fixture (mints a live monomorph, still rejected), not retired.
5. The extended-slice `is_copy`/`contains_reference`/`is_linear` wiring covers
   every concrete-path call site, not only `dup`/`over`/`fill`; both the `dup`
   and bind witnesses build and run.
