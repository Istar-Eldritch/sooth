# P7.S11-follow spec — Check-time monomorphs invisible to the enclosing word

## What this slice fixes

A generic type whose *only* instantiation in a program comes from a
combinator's own signature (the `map`/`and_then`/`wrap` shape P7.S11 exists to
unblock) still does not build end to end. The combinator's own definition
checks, but every site that uses the result fails with `unknown word` on the
generated constructor/accessor, and an eliminator over such a value is rejected
as "ungrounded."

Root cause (validated live against this branch's HEAD, see
[slice11-follow-brief.md](./slice11-follow-brief.md), final two sections, which
supersede all earlier passes in that file): the concrete call-site
`env: HashMap<String, Vec<Overload>>` and the `enums`/`structs` decl slices a
word is checked against are frozen before the per-word loop. A **check-time**
monomorph, minted while checking a word's body, is flushed into the live
`GenericTypes` cell but is invisible to:

1. the constructor/accessor `env` the same word (and later words) look up
   against;
2. id-indexed decl reads (`drop`/layout/`is_copy`/tag, and the eliminator's own
   `gate_decl`/`enum_decl`/`variant_type` reads), which panic or miss on an id
   past the flushed slice;
3. the eliminator's scrutinee classification, frozen at pre-loop
   `eliminator_registry` build time as `Generic` and never re-consulted after a
   later mint.

Two originating mint sites feed this gap, and (per the brief's third pass) both
write into the *same* live `generics_cell` regardless of call shape:

- a combinator's own splice-site output grounding (`inline_combinator`);
- an ordinary mid-word poly call's `apply_subst` mint (no splice, no
  combinator).

Because both mints land in one live cell, a single fallback *at the point of
use* (an `env.get` miss, and id-indexed decl reads) covers both. No
splice-scoped `env` clone, no second wiring site.

## The fix — four parts, one entangled mechanism

### Part 1 — Splice-site output grounding (`src/check/combinators.rs`)

In `inline_combinator` (currently `src/check/combinators.rs:313`), after
`poly_subst` is computed (`:351`) and before the callee body is spliced, force
`apply_subst` over the combinator's declared `sig.outputs` (the same
`apply_subst` already used for inputs at `:680`). This mints the combinator's
own construction of its output monomorph into the live `generics_cell` *ahead*
of the body splice, so the constructor/accessor the body (and, via Part 4, the
enclosing word) resolves already exists in the cell.

This part only forces the mint to happen early enough. It builds no scoped
`env`; the resolution is Part 4's job.

### Part 2 — Id-indexed decl fallback (`src/ast.rs`, `src/check/engine.rs`, `src/check.rs`)

Mirror the existing `GenericTypes::enum_decl` pattern (`src/ast.rs:924`, added
in P7.S12 for exactly this "minted but unflushed" case) with a `struct_decl`
twin over `inst_structs`/`struct_base`. Then add id-indexed decl accessors on
`Ctx` — `with_enum_decl_or_generic` / `with_struct_decl_or_generic` (in
`impl Ctx`, `src/check/engine.rs:1251`) — that return the decl from the flushed
`enums()`/`structs()` slice when the id is in range, else fall back to the live
`generics_cell`'s `enum_decl`/`struct_decl`.

Wire this fallback into every id-indexed lookup that can hit a check-time-only
monomorph outside its home word:

- the `has_drop_overload` drop check at `src/check.rs:1448`;
- the struct drop/layout reads at `src/check.rs:3168-3169`
  (`ctx.structs()[id.index()]`, the golden-10 panic site, out of bounds len 0);
- `check_eliminator_call`'s `gate_decl` read (`src/check.rs:2276`), the enum
  family read (`:2344`), and `enum_decl` read (`:2354`);
- the per-arm `variant_type` computation (`src/check.rs:2426`,
  `variant_type(ctx.enums(), id, *vi)` — route through the fallback so it reads
  the minted decl);
- the body-walk `is_copy`/tag/drop-graph id-indexed reads in `src/check/poly.rs`
  that index `enums`/`structs` directly (the same set P7.S12's note at
  `src/check/poly.rs:417` enumerates), for a mint the current word just made.

Dedup safety is already proven: `instantiate_struct`/`instantiate_enum` dedup by
memo key before minting (`instantiate_struct_dedups_and_counts_from_its_base`,
`src/ast.rs`), so re-grounding the same header never mints a second decl and the
fallback never returns two entries for one monomorph.

### Part 3 — Eliminator scrutinee grounding from the live stack (`src/check/terms.rs`)

Add `scrutinee_enum_id_of_family` in `src/check/terms.rs`: when
`eliminator_registry`'s classification for the call is the frozen `Generic`
entry, read the scrutinee's own concrete `Type::Enum(id, _)` off the live stack
slot (as `check_eliminator_call` already does at `src/check.rs:2334` for the
operative id) instead of trusting the frozen pre-loop classification.

**Constraint (do not ship permissive):** the fallback must confirm the
scrutinee's `id` resolves to a *real, minted* decl via the Part-2
`enum_decl_or_generic` fallback, not merely that the stack type's tag matches
the family. A poly call's own unification can leave a concrete-looking
`Type::Enum` on the stack from substituting `'T = f64` into the type alone,
independent of whether anything grounded that instantiation. Requiring an actual
mint keeps a genuinely-ungrounded non-combinator call getting the honest
"cannot eliminate it while it is ungrounded" diagnostic rather than falling
through to a confusing accessor-not-found error two terms later. (See the
brief's second-pass "item 2" and its third-pass correction: the slice12 fixture
*does* mint mid-word, so with the actual-mint check it correctly grounds; a
fixture that never mints stays honestly rejected.)

### Part 4 — Shared `env`-miss mint fallback (`src/check/terms.rs`)

Add `mint_fallback_candidates(name, ctx)` in `src/check/terms.rs`. On an
ordinary `env.get(name)` miss (the dispatch miss at `src/check/terms.rs:838`),
re-derive `struct_generated_sigs` / `enum_generated_sigs` /
`variant_generated_sigs` fresh from the live `generics_cell`'s still-unflushed
pending mints (`inst_structs`/`inst_enums`, the tail past `struct_base`/
`enum_base`), and return any candidates matching `name`. This is a read-through
computed per-miss; it mutates nothing.

This single fallback covers both originating mints: a splice-local mint (Part 1)
and an ordinary mid-word poly call's own `apply_subst` mint both write into the
same live cell, so both are visible here regardless of call shape. This is the
brief's key third-pass finding: **gate (v) is not a second wiring site**; it is
this one fallback, generic over call shape because it lives at the point of use.

## Growth-structure check (CLAUDE.md)

This touches `src/check/combinators.rs`, `src/check/engine.rs`,
`src/check/terms.rs`, `src/check.rs`, `src/ast.rs`, and id-indexed reads in
`src/check/poly.rs`. That is more than four files, but it does **not** trip the
split/elevate signals: this is one entangled mechanism (a check-time mint made
visible to its enclosing word), not a new subsystem. It *elevates a fallback
pattern already used once* — `GenericTypes::enum_decl` (P7.S12) gets its
`struct_decl` twin, and both are surfaced through `Ctx` accessors at the lowest
common ancestor of their consumers (the id-indexed reads in `check.rs`/`poly.rs`
and the `env`-miss dispatch in `terms.rs`). No file gains a second unrelated
responsibility; no would-be circular dependency forces a split. No module split
or elevation beyond the `Ctx` accessor is warranted. Re-run the signals at phase
exit against `terms.rs`/`combinators.rs` if either grew materially.

## Out of scope (ruled out in the brief, do not widen)

- The standalone-combinator path
  (`ground_into_word_scoped_registries`, `src/check/poly.rs:412`) deliberately
  mints into a dropped clone and must not reach the live `env`. Unchanged.
- `poly_env` (the polymorphic-word table): a poly word's signature never depends
  on a concrete monomorph pre-existing in `env`. Unchanged.
- `check_extern_decls` / `check_main_effect` and any pre-loop pass reading an
  `env` snapshot: they run before the per-word loop over parsed source text,
  which can only name a type that already exists. No gap.
- The first (retracted) brief's "re-run the three generated-sig helpers over
  each word's post-flush tail" fix: refuted — at the failing lookup nothing has
  minted yet; resolving the constructor is itself what triggers the mint. Do not
  implement the end-of-loop-flush projection.

## Test plan

### Unit tests (beside each stage function, `thing_condition_expected`)

- `src/ast.rs`: `struct_decl` returns the unflushed mint for an id past
  `struct_base` and `None` for a flushed/hand-written id
  (`generic_types_struct_decl_reads_an_unflushed_mint`,
  `generic_types_struct_decl_none_for_a_flushed_id`), mirroring the existing
  `enum_decl` coverage.
- `src/check/engine.rs`: `with_enum_decl_or_generic` /
  `with_struct_decl_or_generic` return the flushed decl when in range and the
  live-cell decl when past it
  (`with_struct_decl_or_generic_falls_back_to_the_live_cell`,
  `with_enum_decl_or_generic_prefers_the_flushed_slice`).
- `src/check/terms.rs`:
  - `mint_fallback_candidates_finds_an_unflushed_constructor` and
    `mint_fallback_candidates_empty_when_nothing_pending` (happy + edge).
  - `scrutinee_enum_id_of_family_reads_a_minted_scrutinee` and
    `scrutinee_enum_id_of_family_none_when_type_looks_concrete_but_unminted`
    (the actual-mint constraint, the Part-3 permissiveness guard).
- `src/check/combinators.rs`: `inline_combinator_grounds_its_declared_outputs`
  (a combinator whose output monomorph is minted before the splice) plus the
  existing error/edge case coverage.

Mutation-test each new guard: confirm deleting the Part-3 actual-mint check
re-breaks the slice12 honest-diagnostic assertion, and deleting Part 1's output
grounding re-breaks golden 6, so no test is a placebo.

### Golden tests (exit criteria)

These four goldens already exist and pin the bug as expected behavior; migrate
them **as part of this slice**, flipping the assertion and rewriting the doc
comment to describe the now-fixed behavior (not the bug). This is a separately
scoped item from the fix mechanism.

`tests/phase7_slice11.rs`:

- `a_check_time_monomorphs_constructors_are_absent_from_the_call_site_env`
  (golden 6, enum + `Result?` eliminator): flip from
  `build_error` / `unknown word \`Ok\` in \`main\`` to
  `build_and_run` / exit 0.
- `a_check_time_struct_monomorphs_constructor_is_absent_from_the_call_site_env`
  (golden 10, struct + `drop`): flip from
  `build_error` / `unknown word \`Cell\` in \`main\`` to
  `build_and_run` / exit 0.
- `a_standalone_mint_after_an_earlier_check_time_mint_lands_at_the_right_id`
  (golden 9, stale-base): flip from
  `build_error` / `unknown word \`Ok\` in \`main\`` to
  `build_and_run` / exit 0.

Rename the tests if their `_are_absent`/`_is_absent` names now assert the
opposite (e.g. `..._resolves_at_the_call_site`), and update each doc comment
accordingly.

`tests/phase7_slice12.rs`:

- `concrete_body_generic_eliminator_message_does_not_fabricate_an_instantiation`
  (`src/check/../tests/phase7_slice12.rs:606`): this fixture's `wrap` is an
  ordinary non-inline poly word, so it mints `Pair[f64]` mid-word through the
  ordinary poly-call path — a real check-time mint, now correctly grounded. Flip
  from `build_error` to `build_and_run` / exit 0. **The fixture additionally has
  its own independent, unrelated stack-linearity bug**: its `Nil` arm
  `~[ ( Nil ) 0.0 ]` never consumes the `Nil` variant it is given (the `One` arm
  consumes via `One>`). Fix the fixture source (e.g. `~[ ( Nil ) drop 0.0 ]`) so
  both arms are stack-balanced before it can assert a clean exit 0. Do not
  conflate this fixture bug with the mechanism being fixed; document it in the
  migrated doc comment as an orthogonal fixture fix.

Verify goldens that must **not** flip stay green:
`an_intermediate_construction_over_a_different_header_is_still_unknown`
(golden 7 — an unrelated header the signature never mentions stays
`unknown word \`One\` in \`wrap\``), golden 6b (enum +`drop`, no eliminator),
golden 8, and slice12's
`a_generic_eliminator_in_a_standalone_checked_combinator_is_rejected` /
`a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire`.

### Full-suite regression

`cargo test --no-fail-fast` (all targets) must show exactly the four expected
flips above and no unrelated regressions (the brief's third pass validated this
count end to end). "Green" gate: `cargo fmt --check && cargo clippy -- -D
warnings && cargo test`.

## Delivery plan

**Reasoning on the split.** Parts 1-4 are one entangled mechanism (the
growth-structure note above), but they are *not* one atomic green step. Part 2 is
pure infrastructure: a `struct_decl` twin plus `Ctx` fallback accessors wired
into id-indexed reads. On its own it changes no observable behavior, because
without Parts 1/4 the constructor lookup still fails *before* any id-indexed read
is reached for a check-time-only monomorph, so no golden flips and the tree stays
green. Parts 1, 3, 4 are the behavior change: they make the mint reachable, and
the moment they land the pre-existing bug-pinning goldens turn red (they assert
`build_error`, which is no longer true). So the test migration cannot be a
separate green phase *after* the behavior lands — it must land in the same phase.
Hence two phases: infrastructure (green, no flips), then behavior + migration
(atomic flip). Part 3's actual-mint guard depends on Part 2's
`enum_decl_or_generic`, so Part 2 must precede it; this ordering falls out for
free.

### Phase 1 — id-indexed decl fallback infrastructure

- **Goal.** Land Part 2 with no behavior change: a `struct_decl` twin and `Ctx`
  fallback accessors, wired into every id-indexed decl read that a check-time
  mint can reach, proven safe by staying green with no golden flips.
- **File scope.** `src/ast.rs` (`struct_decl` twin of `enum_decl` at `:924`,
  over `inst_structs`/`struct_base`); `src/check/engine.rs`
  (`with_enum_decl_or_generic` / `with_struct_decl_or_generic` in `impl Ctx`,
  `:1251`); `src/check.rs` (`:1448`, `:2276`, `:2344`, `:2354`, `:2426`,
  `:3168-3169`); `src/check/poly.rs` (the id-indexed `is_copy`/tag/drop-graph
  reads the note at `:417` enumerates).
- **Entry conditions.** Tree green at HEAD.
- **Exit criteria.** New unit tests pass —
  `generic_types_struct_decl_reads_an_unflushed_mint`,
  `generic_types_struct_decl_none_for_a_flushed_id` (`src/ast.rs`);
  `with_struct_decl_or_generic_falls_back_to_the_live_cell`,
  `with_enum_decl_or_generic_prefers_the_flushed_slice`
  (`src/check/engine.rs`). Full suite green, **zero golden flips** (goldens 6, 9,
  10 still assert `build_error`; golden 10 no longer *panics* but still errors at
  the constructor lookup). `cargo fmt --check && cargo clippy -- -D warnings`
  clean.
- **Effort / difficulty.** Small-to-medium effort; medium difficulty (mechanical
  twin + routing, but the poly.rs read set must be enumerated exactly).

### Phase 2 — check-time mint resolution and test migration

- **Goal.** Land Parts 1, 3, 4 so a check-time-only monomorph resolves in its
  enclosing word, and migrate the four bug-pinning fixtures in the same green
  step.
- **File scope.** `src/check/combinators.rs` (Part 1: `apply_subst` over
  `sig.outputs` in `inline_combinator`, after `:351`, before the body splice);
  `src/check/terms.rs` (Part 3: `scrutinee_enum_id_of_family` with the
  actual-mint guard; Part 4: `mint_fallback_candidates` at the `env.get` miss,
  `:838`); `tests/phase7_slice11.rs` (three flips); `tests/phase7_slice12.rs`
  (one flip plus the independent `Nil`-arm linearity fixture fix).
- **Entry conditions.** Phase 1 merged and green.
- **Exit criteria.** The three `phase7_slice11.rs` goldens (6, 9, 10) and the
  `phase7_slice12.rs` fixture all `build_and_run` to exit 0, with flipped
  assertions and rewritten doc comments; goldens 7, 6b, 8 and slice12's two
  standalone-rejection tests stay red-as-designed. New unit tests pass —
  `mint_fallback_candidates_finds_an_unflushed_constructor`,
  `mint_fallback_candidates_empty_when_nothing_pending`,
  `scrutinee_enum_id_of_family_reads_a_minted_scrutinee`,
  `scrutinee_enum_id_of_family_none_when_type_looks_concrete_but_unminted`
  (`src/check/terms.rs`); `inline_combinator_grounds_its_declared_outputs`
  (`src/check/combinators.rs`). Each new guard proven non-placebo by mutation
  (deleting the Part-3 actual-mint check re-breaks slice12; deleting Part 1's
  output grounding re-breaks golden 6). Full green gate, exactly the four flips,
  no other suite changes.
- **Effort / difficulty.** Medium effort; **hard** — the splice-site grounding,
  the scrutinee actual-mint guard (permissiveness is the trap), the shared
  `env`-miss fallback, and four coupled migrations must land atomically green.

## Phases (JSON)

```json
[
  {
    "phase": 1,
    "focus": "Id-indexed decl fallback infrastructure: struct_decl twin in src/ast.rs, with_enum_decl_or_generic / with_struct_decl_or_generic in src/check/engine.rs, wired into the id-indexed decl reads in src/check.rs and src/check/poly.rs. No behavior change; tree stays green with zero golden flips. Unit tests for struct_decl and the Ctx accessors.",
    "effort": "small-medium",
    "difficulty": "medium"
  },
  {
    "phase": 2,
    "focus": "Check-time mint resolution: splice-site sig.outputs grounding in inline_combinator (src/check/combinators.rs), scrutinee_enum_id_of_family with an actual-mint guard and mint_fallback_candidates at the env.get miss (src/check/terms.rs). Migrate the four bug-pinning fixtures atomically (three flips in tests/phase7_slice11.rs, one flip plus an independent Nil-arm linearity fix in tests/phase7_slice12.rs). Unit + mutation tests for each new guard.",
    "effort": "medium",
    "difficulty": "hard"
  }
]
```

## Acceptance criteria

1. Golden 6, 6b, 8, 9, 10 (`tests/phase7_slice11.rs`) and slice12's migrated
   fixture all build and run to exit 0; the four migrated tests assert
   `build_and_run` with updated doc comments; goldens 7 and the two slice12
   standalone-rejection tests stay red-as-designed (honest diagnostics).
2. Each new guard has beside-it unit coverage and is proven non-placebo by
   mutation.
3. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green,
   with exactly the four documented golden flips and no other suite changes.
