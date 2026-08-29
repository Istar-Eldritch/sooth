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

In `inline_combinator` (`src/check/combinators.rs:313`, the `fn` signature),
after `poly_subst` is computed (`:351`) and before the callee body is spliced,
force `apply_subst` over the combinator's declared `sig.outputs` (the same
`apply_subst` already used for the quotation-input parameters at `:680`). This
mints the combinator's own construction of its output monomorph into the live
`generics_cell` *ahead* of the body splice, so the constructor/accessor the body
(and, via Part 4, the enclosing word) resolves already exists in the cell.

This part only forces the mint to happen early enough. It builds no scoped
`env`; the resolution is Part 4's job.

**Error-handling ruling (`apply_subst` is fallible).** `apply_subst`
(`src/check/poly.rs:8142`) raises `poly_unbound_output_error` /
`poly_unbound_output_ty_error` (`src/check/poly.rs:9271`/`:9286`) when a
declared output type variable is bound by no input. Forcing `sig.outputs`
through it early moves *where* that `Err` can first fire. The ruling: **propagate
the `Err`** (do not discard it — a silently-swallowed unbound output would let a
broken signature splice a garbage monomorph). Golden 5
(`an_output_only_type_variable_is_still_uncallable`,
`tests/phase7_slice11.rs:161`) stays green under this: it asserts the *substring*
``has output variable `'U` that no input binds`` (`poly_unbound_output_error`'s
text, `src/check/poly.rs:9275`), and Part 1 raises that identical helper with
`name = "wrap"` and the call-site span, so the substring still matches
regardless of the callee/where prefix. Golden 5's *explicit* case
(`wrap[i64 i64]`) is also unaffected: the `takes no type arguments` rejection
(`no_type_arguments_error`, `src/check.rs:1364`) fires at dispatch *before*
`inline_combinator` is entered, so Part 1's early `apply_subst` never runs for
it. Verify both assertions still hold when Part 1 lands (they are substring
checks on helpers Part 1 does not modify).

### Part 2 — Id-indexed decl fallback (`src/ast.rs`, `src/check/engine.rs`, `src/check.rs`)

Mirror the existing `GenericTypes::enum_decl` pattern (`src/ast.rs:924`, added
in P7.S12 for exactly this "minted but unflushed" case) with a `struct_decl`
twin over `inst_structs`/`struct_base`. Then add id-indexed decl accessors on
`Ctx` — `with_enum_decl_or_generic` / `with_struct_decl_or_generic` (in
`impl Ctx`, `src/check/engine.rs:1251`).

**Accessor shape (the `RefCell` constrains it).** `Ctx::generics()` returns
`Option<&RefCell<GenericTypes>>` (`src/check/engine.rs:1344`), so the fallback
decl lives behind a `RefCell` borrow and *cannot* be returned as an
`Option<&EnumDecl>` outliving that borrow. The accessor is therefore
**closure-taking**:

```rust
fn with_enum_decl_or_generic<R>(&self, id: EnumId, f: impl FnOnce(Option<&EnumDecl>) -> R) -> R
fn with_struct_decl_or_generic<R>(&self, id: StructId, f: impl FnOnce(Option<&StructDecl>) -> R) -> R
```

Each: if `id.index() < self.enums()/structs().len()`, call `f(Some(&slice[..]))`;
else borrow `generics()` (holding the `Ref` alive across the `f` call) and
call `f(self.generics().and_then(|c| c.borrow().enum_decl(id)))` — i.e. the
live-cell `enum_decl`/`struct_decl` twin, `None` when the id names nothing
pending. Call sites that need only a boolean (Part 3's actual-mint check) pass
`|d| d.is_some()`; call sites that need a value (`variant_type`) compute it
inside the closure. Use this **one** name spelling everywhere
(`with_enum_decl_or_generic`), including in Part 3.

Wire this fallback into every id-indexed lookup on the **concrete checker path**
(`src/check.rs`) that can hit a check-time-only monomorph outside its home word:

- the `has_drop_overload` read inside `cannot_copy_error`
  (`src/check.rs:1448` — note this is a *diagnostic-message helper*, not the
  live drop check, but it still indexes `ctx.structs()` and so needs the
  fallback to avoid an out-of-bounds on a check-time-only mint);
- the struct drop/layout reads at `src/check.rs:3168-3169`
  (`ctx.structs()[id.index()]`, the golden-10 panic site once Parts 1/4 land,
  out of bounds on len-0);
- `check_eliminator_call`'s `gate_decl` read (`src/check.rs:2276`), the enum
  family read (`:2344`), the `enum_decl` read (`:2354`), and the operative-id
  scrutinee read (`:2335`, `let Type::Enum(id, _) = referent`);
- the per-arm `variant_type` computation (`src/check.rs:2426`,
  `variant_type(ctx.enums(), id, *vi)`). **Do not change `variant_type`'s
  signature** (`src/ast.rs:528`, `variant_type(&[EnumDecl], EnumId, usize)` —
  it indexes a slice and cannot be handed a single fallback decl). Instead
  rewrite the *call site* to branch through `with_enum_decl_or_generic`,
  computing the `Type::Variant(id, vi, display_static)` inside the closure —
  exactly the shape `src/check/poly.rs:8320` already uses for the past-`len`
  display-name fallback.

**`is_copy` / `contains_reference` are not wired here.** `is_copy`
(`src/check/builtins.rs:228`, `pub fn is_copy(ty, &[StructDecl], &[EnumDecl],
&[ArrayDecl])`) takes no `Ctx`, recurses over fields, and its `src/check/poly.rs`
call sites (`:959`, `:1545`, `:2392`, `:5073`, …) already receive the body
walk's own `structs`/`enums` slice parameters. Those are the **standalone /
poly-body-walk path**, which `ground_into_word_scoped_registries`
(`src/check/poly.rs:425`) already covers by flushing this batch's mints into a
local `local.enums`/`local.structs` clone before the walk (the note at
`src/check/poly.rs:417`, which names four reads: `is_copy`,
`contains_reference`, the drop graph, and `tag`). Do **not** change `is_copy`'s
signature and do **not** thread `Ctx` into the poly walk. The direct
`enums[id.index()]` reads inside that walk at `:1807` (`tag`), `:3243`
(`gate_decl`), and `:3331` (family-name compare) are on that same
already-flushed path; `:3464` and `:8320` are *already* fallback-guarded by
P7.S12. Part 2 touches none of these.

**Footgun — do not touch the generic *header* table.** `generics.enums[idx]` /
`generics.structs[idx]` (`src/check/poly.rs:3260`, `:3498-3500`, `:4558`,
`:4638-4699`) index a *different* registry — the un-instantiated generic header
table — keyed by generic-decl index, not `EnumId`. The names are one character
apart from the `inst_enums`/`inst_structs` mint reads. Rerouting any of these
through the id-indexed fallback is a bug; leave them all untouched.

Dedup safety is already proven: `instantiate_struct`/`instantiate_enum` dedup by
memo key before minting (`instantiate_struct_dedups_and_counts_from_its_base`,
`src/ast.rs`), so re-grounding the same header never mints a second decl and the
fallback never returns two entries for one monomorph.

### Part 3 — Eliminator scrutinee grounding from the live stack (`src/check/terms.rs`)

The frozen-`Generic`-classification rejection lives at
`src/check/terms.rs:617-618`:

```rust
let EliminatorTarget::Concrete(enum_id) = target else {
    return Err(concrete_body_generic_eliminator_error(ctx, span, name));
};
```

Add `scrutinee_enum_id_of_family` in `src/check/terms.rs`: when the classification
is the frozen `Generic` entry (the else-arm above), instead of erroring
immediately, recover the scrutinee's own concrete `Type::Enum(id, _)` and, if it
resolves to a real mint, proceed with that `id`.

**The scrutinee read is not `stack.last()`.** At `terms.rs:617` the arms are
still stacked on top of the scrutinee; the scrutinee slot is only reached after
the variable-arity arm-collection scan that `check_eliminator_call` runs later
(`src/check.rs:2285-2302`: a `while let` that pops tagged quotation literals
until the first non-arm operand, then reads the scrutinee at `:2305`). Part 3
therefore does **not** peek `stack.last()` at `:617`; it replicates that same
arm-collection scan (stop at the first operand that is not a tagged quotation
literal; that operand is the scrutinee) to locate the scrutinee slot, then reads
its `Type::Enum(id, _)` (unwrapping a `ref_parts` referent exactly as
`src/check.rs:2335` does). Factor the scan so both sites share it rather than
duplicating the loop.

**Constraint (do not ship permissive):** the fallback must confirm the
scrutinee's `id` resolves to a *real, minted* decl via Part 2's
`with_enum_decl_or_generic` (passing `|d| d.is_some()`), not merely that the
stack type's tag matches the family. A poly call's own unification can leave a
concrete-looking `Type::Enum` on the stack from substituting `'T = f64` into the
type alone, independent of whether anything grounded that instantiation.
Requiring an actual mint keeps a genuinely-ungrounded non-combinator call
getting the honest "cannot eliminate it while it is ungrounded" diagnostic
rather than falling through to a confusing accessor-not-found error two terms
later. (See the brief's second-pass "item 2" and its third-pass correction: the
slice12 fixture *does* mint mid-word, so with the actual-mint check it correctly
grounds; a fixture that never mints stays honestly rejected.)

**Witness caveat (see the test plan).** After the slice12 flip, *no integration
test witnesses this actual-mint check's deletion*: the flipped fixture mints and
grounds either way, and both surviving rejection fixtures use an `i64` stand-in
scrutinee that never reaches `Type::Enum`. The check is therefore treated as
defensive coding, witnessed only by a hand-built-`Ctx` unit test; the test plan
does not claim a mutation-test for it. See the test-plan ruling under
"Unit tests" and "Phase 2 exit criteria".

### Part 4 — Shared `env`-miss mint fallback (`src/check/terms.rs`)

Add `mint_fallback_candidates(name, ctx)` in `src/check/terms.rs`. On an
ordinary `env.get(name)` miss (the dispatch miss at `src/check/terms.rs:838`),
re-derive the generated constructor/accessor sigs for the live cell's
still-unflushed pending mints and return any matching `name`. This is a
read-through computed per-miss; it mutates nothing.

**The id-derivation must run over the *extended* slice, never the pending tail
alone.** `struct_generated_sigs`/`enum_generated_sigs`/`variant_generated_sigs`
(`src/check/declarations.rs:1716`/`:1753`/`:1786`) compute each decl's own
`Type::Struct(StructId::from_index(idx), ..)` /
`Type::Enum(EnumId::from_index(idx), ..)` from `enumerate()` over *whatever
slice they are handed*. Run one of them over just the pending tail and every
candidate is minted at `from_index(0..)` — a wrong, colliding id.
`enum_generated_sigs_over_an_extended_slice_carries_the_monomorphs_own_id`
(`src/check/declarations.rs:4053`) exists specifically to forbid that shape.
So `mint_fallback_candidates` mirrors the live precedent at
`src/check/poly.rs:606-625`: build the **extended** slice (the flushed
`ctx.enums()` ++ the live cell's unflushed `inst_enums`, and the struct/variant
twins), run the helper over the concatenation, then
`.skip(enum_generated_sigs(ctx.enums()).len())` (resp. the struct/variant
skip). The skip length is stable because both runs are one in-order
`enumerate().push()` loop over a shared prefix
(`enum_generated_sigs_prefix_is_stable_under_extension`,
`src/check/declarations.rs:4092`). Filter the skipped tail's sigs by `name`.

**Field access.** `struct_base`/`enum_base` are *private*
(`src/ast.rs:633-634`); `inst_structs`/`inst_enums` are `pub`
(`src/ast.rs:593-594`). Since Part 4 lives in `src/check/terms.rs`,
`mint_fallback_candidates` works entirely off the **public** `inst_structs`/
`inst_enums` fields (concatenated onto `ctx.enums()`/`ctx.structs()` to form the
extended slice) — it needs no access to the private bases, because the skip
length is derived from `ctx.enums()`/`ctx.structs()` (the flushed prefix), not
from a base. No new accessor on `GenericTypes` is required for Part 4.

This single fallback covers both originating mints: a splice-local mint (Part 1)
and an ordinary mid-word poly call's own `apply_subst` mint both write into the
same live cell, so both are visible here regardless of call shape. This is the
brief's key third-pass finding: **gate (v) is not a second wiring site**; it is
this one fallback, generic over call shape because it lives at the point of use.

**Name-collision caveat.** `mint_fallback_candidates` returns *all* pending
mints whose surface name matches. Variant-ctor env keys are module-blind (a
known property: `Less`/`Equal`/`Greater` and friends are reserved across
modules), so two pending mints in one live cell could in principle generate the
same surface name (two generic headers each with an identically-named variant,
or one variant reached from two modules). The test plan adds a probe for this
(see "Unit tests"); if it turns out unreachable from a real single-cell program,
the probe records that with a stated reason rather than shipping silent.

## Growth-structure check (CLAUDE.md)

This touches `src/check/combinators.rs`, `src/check/engine.rs`,
`src/check/terms.rs`, `src/check.rs`, and `src/ast.rs`. (It reads but does not
modify `src/check/poly.rs` — its id-indexed body-walk reads are the standalone
path already covered by `ground_into_word_scoped_registries`; see Part 2.) That
is more than four files, but it does **not** trip the split/elevate signals:
this is one entangled mechanism (a check-time mint made visible to its enclosing
word), not a new subsystem. It *elevates a fallback pattern already used once* —
`GenericTypes::enum_decl` (P7.S12) gets its `struct_decl` twin, and both are
surfaced through `Ctx` accessors at the lowest common ancestor of their
consumers (the id-indexed reads in `check.rs` and the `env`-miss dispatch in
`terms.rs`). No file gains a second unrelated
responsibility; no would-be circular dependency forces a split. No module split
or elevation beyond the `Ctx` accessor is warranted. Re-run the signals at phase
exit against `terms.rs`/`combinators.rs` if either grew materially.

## Out of scope (ruled out in the brief, do not widen)

- The standalone-combinator path
  (`ground_into_word_scoped_registries`, `src/check/poly.rs:425`, the `fn`;
  `:412` is mid-doc-comment) deliberately mints into a dropped clone and must
  not reach the live `env`. Unchanged.
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
  - `mint_fallback_candidates_at_a_colliding_name_returns_both` (or, if a
    single live cell provably cannot hold two same-surface-name pending mints,
    this test asserts that impossibility with the reason stated in the doc
    comment): the module-blind-variant-key collision probe for Part 4. Build a
    hand-crafted `GenericTypes` with two pending `inst_enums` whose variants
    render the same surface name and assert `mint_fallback_candidates` returns
    both overloads (so a later ambiguity/last-write bug would be visible), *or*
    document why the parse/instantiation path cannot produce that state.
  - `scrutinee_enum_id_of_family_reads_a_minted_scrutinee` and
    `scrutinee_enum_id_of_family_none_when_type_looks_concrete_but_unminted`
    (the actual-mint constraint, the Part-3 permissiveness guard). These are
    hand-built-`Ctx` unit tests. **They are the *only* witness for the Part-3
    actual-mint check** — see the mutation-test ruling below.
- `src/check/combinators.rs`: `inline_combinator_grounds_its_declared_outputs`
  (a combinator whose output monomorph is minted before the splice) plus the
  existing error/edge case coverage.

**Mutation-test ruling (corrects an unpassable earlier criterion).** Deleting
Part 1's output grounding re-breaks golden 6 (now flipped to `build_and_run`),
so Part 1's guard is genuinely non-placebo at integration level; keep that
mutation check. **But the Part-3 actual-mint check has no integration witness**:
after the slice12 flip, that fixture mints and grounds whether or not the guard
is present (its `wrap` genuinely mints `Pair[f64]` mid-word), and the two
surviving rejection fixtures
(`a_generic_eliminator_in_a_concrete_body_is_rejected`,
`a_generic_eliminator_in_a_standalone_checked_combinator_is_rejected`,
`tests/phase7_slice12.rs`) both use an `i64` stand-in scrutinee that never
reaches `Type::Enum`, so neither can observe the guard's deletion either. No
real program can present a concrete-looking-but-never-minted `Type::Enum`
scrutinee — producing an enum *value* requires constructing it, which mints it —
so the guard is **defensive coding**, witnessed only by the hand-built-`Ctx`
unit test `scrutinee_enum_id_of_family_none_when_type_looks_concrete_but_unminted`.
The earlier "deleting the Part-3 actual-mint check re-breaks the slice12
honest-diagnostic assertion" mutation claim was false and is removed from the
test plan and both exit criteria. (Residual risk: if a witnessing program is
ever found, re-home an integration mutation-test onto it.)

### Golden tests (exit criteria)

Four goldens already exist and pin the bug as expected behavior; migrate them
**as part of this slice**, flipping the assertion and rewriting the doc comment
to describe the now-fixed behavior (not the bug). One further test (golden 6b)
is *added* new. This is a separately scoped item from the fix mechanism.

`tests/phase7_slice11.rs`:

- `a_check_time_monomorphs_constructors_are_absent_from_the_call_site_env`
  (golden 6, enum + `Result?` eliminator): flip from
  `build_error` / `unknown word \`Ok\` in \`main\`` to `build_and_run`, and
  **assert stdout**, not just exit code. Golden 6's source is golden 1's minus
  `mki`, so it prints the same value:`assert_eq!(stdout, "8\n")` (a bare
  exit-0 check would pass even if the eliminator resolved the wrong arm or the
  wrong monomorph's accessor).
- `a_check_time_struct_monomorphs_constructor_is_absent_from_the_call_site_env`
  (golden 10, struct + `drop`): flip from
  `build_error` / `unknown word \`Cell\` in \`main\`` to `build_and_run` /
  exit 0. Its `main` is `… wrap drop` with no `.`, so it prints nothing;
  exit-0-only is the correct assertion here.
- `a_standalone_mint_after_an_earlier_check_time_mint_lands_at_the_right_id`
  (golden 9, stale-base): flip from
  `build_error` / `unknown word \`Ok\` in \`main\`` to `build_and_run` /
  exit 0. Its `main` is `seed 7 ~[ 1 add ] wrap drop` — no eliminator, no `.`,
  so it prints nothing; exit-0-only is correct here too.

Rename the tests if their `_are_absent`/`_is_absent` names now assert the
opposite (e.g. `..._resolves_at_the_call_site`), and update each doc comment
accordingly.

**Add golden 6b (a new committed test, not a flip).** There is no `6b` in the
tree today (`grep -n 6b tests/phase7_slice11.rs` returns nothing); it existed
only as brief prose. Add it as a real test: golden 6's fixture with the
eliminator dropped — enum construction + `drop`, no `Result?`:

```sooth
type: Result['T 'E] | Ok 'T | Err 'E ;
: wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;
: main ( -- ) 7 ~[ 1 add ] wrap drop ;
```

Name it e.g. `a_check_time_enum_monomorph_constructs_and_drops_without_an_eliminator`,
asserting `build_and_run` / exit 0 (no stdout). It is independently valuable: it
is the only enum-shaped test that exercises Part 4's constructor fallback and
Part 2's enum drop/layout reads *without* also invoking Part 3's eliminator
machinery (golden 10 gives that isolation for structs; goldens 6 and 9 both also
route through the eliminator). Being a new test born asserting the fixed
behavior, it is **not** in any flip set and **not** in the must-not-flip set; it
lands in Phase 2 alongside the flips (it would be red before Parts 1/4).

`tests/phase7_slice12.rs`:

- `concrete_body_generic_eliminator_message_does_not_fabricate_an_instantiation`
  (`tests/phase7_slice12.rs:606`): this fixture's `wrap` is an
  ordinary non-inline poly word, so it mints `Pair[f64]` mid-word through the
  ordinary poly-call path — a real check-time mint, now correctly grounded. Flip
  from `build_error` to `build_and_run`, and **assert stdout**: the `One` arm
  does `One>` and the trailing `.` prints the unwrapped `f64`, so assert the
  value the program prints (verify the exact `f64` rendering during impl, e.g.
  `"7.5\n"`). **The fixture additionally has its own independent, unrelated
  stack-linearity bug**: its `Nil` arm `~[ ( Nil ) 0.0 ]` never consumes the
  `Nil` variant it is given (the `One` arm consumes via `One>`). Fix the fixture
  source (e.g. `~[ ( Nil ) drop 0.0 ]`) so both arms are stack-balanced before
  it can assert a clean exit 0. Do not conflate this fixture bug with the
  mechanism being fixed; document it in the migrated doc comment as an
  orthogonal fixture fix.

**Accepted, named coverage loss (item: the fabricated-instantiation regression
guard).** The test above is the *sole* place in the tree asserting
`!err.contains("nothing in this program instantiates")` and `!err.contains("i64")`
— its `f64`-vs-`i64` discriminator proves the honest-diagnostic path does not
fabricate a specific instantiation when a monomorph *does* exist. Flipping it to
`build_and_run` retires that guard. It **cannot be re-homed**: the property it
guards is "a monomorph exists yet the message must not claim none / must not
fabricate one", and any fixture where a monomorph exists now *builds* (that is
this slice's whole point), while every fixture that stays rejected
(`a_generic_eliminator_in_a_concrete_body_is_rejected` and the standalone twin,
both `i64` stand-ins) genuinely instantiates nothing — so "nothing in this
program instantiates" would be *true* there and the negative assertion would be
meaningless. This regression guard is therefore **retired as an accepted, named
tradeoff of this slice**, not silently dropped. (Residual risk: a future
reintroduction of a fabricated-instantiation string in
`concrete_body_generic_eliminator_error` would no longer be caught by an
integration test.)

Verify tests that must **not** flip stay green (an explicit regression set):

- `an_intermediate_construction_over_a_different_header_is_still_unknown`
  (golden 7 — an unrelated header the signature never mentions stays
  `unknown word \`One\` in \`wrap\``): stays`build_error`, unchanged.
- `a_body_indexing_its_grounded_output_decl_reports_not_panics`
  (golden 4, `tests/phase7_slice11.rs:141`): stays `build_error`; its message
  embeds `Result[i64 i64]` produced via an id-indexed enum-decl read (`tag`)
  that Part 2 reroutes through the fallback — assert the message is unchanged
  so the reroute is transparent, not perturbing.
- `an_output_only_type_variable_is_still_uncallable`
  (golden 5, `tests/phase7_slice11.rs:161`): stays `build_error`; guards Part
  1's error-propagation ruling (see Part 1).
- `a_combinator_returning_an_array_of_its_grounded_monomorph_builds_and_does_not_panic`
  (golden 8, `tests/phase7_slice11.rs:242`): this is **not** a rejection case
  and **not** in any flip set — it is an existing, always-green,
  `build_and_run` / exit-0 compile-only witness (`hold` is never called). It
  must simply keep passing; do not frame it as "stays red".
- slice12's
  `a_generic_eliminator_in_a_standalone_checked_combinator_is_rejected` and
  `a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire`:
  stay `build_error` (honest diagnostics).
- slice12's `a_generic_eliminator_in_a_concrete_body_is_rejected`
  (`tests/phase7_slice12.rs:543`): stays `build_error` — the direct un-minted
  control for the flipped slice12 fixture. Same message and gate, differing
  only in whether the scrutinee mints, so it is the single best evidence the
  Part-3 actual-mint check *discriminates* rather than being blanket-permissive
  (though note: it uses an `i64` stand-in scrutinee, so it does not by itself
  witness the guard's deletion — see the mutation-test ruling above).

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
`with_enum_decl_or_generic`, so Part 2 must precede it; this ordering falls out for
free.

### Phase 1 — id-indexed decl fallback infrastructure

- **Goal.** Land Part 2 with no behavior change: a `struct_decl` twin and `Ctx`
  fallback accessors, wired into every id-indexed decl read that a check-time
  mint can reach, proven safe by staying green with no golden flips.
- **File scope.** `src/ast.rs` (`struct_decl` twin of `enum_decl` at `:924`,
  over `inst_structs`/`struct_base`); `src/check/engine.rs`
  (`with_enum_decl_or_generic` / `with_struct_decl_or_generic` in `impl Ctx`,
  `:1251`, closure-taking — see Part 2); `src/check.rs` (the concrete-path
  reads: `:1448` inside `cannot_copy_error`, `:2276`, `:2335`, `:2344`,
  `:2354`, `:2426`, `:3168-3169`). **`src/check/poly.rs` is not modified** —
  its id-indexed body-walk reads are the standalone path already covered by
  `ground_into_word_scoped_registries` (Part 2); do not touch the
  `generics.enums[idx]` header table (`:3260`, `:3498-3500`, `:4558`,
  `:4638-4699`).
- **Entry conditions.** Tree green at HEAD.
- **Exit criteria.** New unit tests pass —
  `generic_types_struct_decl_reads_an_unflushed_mint`,
  `generic_types_struct_decl_none_for_a_flushed_id` (`src/ast.rs`);
  `with_struct_decl_or_generic_falls_back_to_the_live_cell`,
  `with_enum_decl_or_generic_prefers_the_flushed_slice`
  (`src/check/engine.rs`). Full suite green, **zero golden flips**: goldens 6,
  9, 10 still assert `build_error` with their existing messages, unchanged.
  (Golden 10 does **not** panic at HEAD — the `ctx.structs()[id.index()]` panic
  is only reachable once Parts 1/4 land in Phase 2 — so Phase 1 makes no claim
  about a panic disappearing; it only requires golden 10's existing
  `unknown word \`Cell\`` assertion to keep passing untouched.)
  `cargo fmt --check && cargo clippy -- -D warnings` clean.
- **Effort / difficulty.** Small-to-medium effort; medium difficulty (mechanical
  twin + routing of the `src/check.rs` reads through the closure accessor).

### Phase 2 — check-time mint resolution and test migration

- **Goal.** Land Parts 1, 3, 4 so a check-time-only monomorph resolves in its
  enclosing word, and migrate the four bug-pinning fixtures in the same green
  step.
- **File scope.** `src/check/combinators.rs` (Part 1: `apply_subst` over
  `sig.outputs` in `inline_combinator`, after `:351`, before the body splice);
  `src/check/terms.rs` (Part 3: `scrutinee_enum_id_of_family` with the
  actual-mint guard; Part 4: `mint_fallback_candidates` at the `env.get` miss,
  `:838`); `tests/phase7_slice11.rs` (three flips plus the new golden 6b);
  `tests/phase7_slice12.rs` (one flip plus the independent `Nil`-arm linearity
  fixture fix).
- **Entry conditions.** Phase 1 merged and green.
- **Exit criteria.** The three `phase7_slice11.rs` goldens (6, 9, 10) and the
  `phase7_slice12.rs` fixture all `build_and_run` to exit 0, with flipped
  assertions and rewritten doc comments (golden 6 and the slice12 fixture
  additionally assert stdout, not just exit code — see the golden-test plan);
  the new golden 6b (added here, born green) `build_and_run`s to exit 0;
  golden 7 and slice12's `a_generic_eliminator_in_a_concrete_body_is_rejected`,
  `a_generic_eliminator_in_a_standalone_checked_combinator_is_rejected`,
  `a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire` stay
  `build_error`; goldens 4, 5, 8 stay green unchanged (golden 8 is an existing
  accept test, not a rejection). New unit tests pass —
  `mint_fallback_candidates_finds_an_unflushed_constructor`,
  `mint_fallback_candidates_empty_when_nothing_pending`,
  the Part-4 name-collision probe,
  `scrutinee_enum_id_of_family_reads_a_minted_scrutinee`,
  `scrutinee_enum_id_of_family_none_when_type_looks_concrete_but_unminted`
  (`src/check/terms.rs`); `inline_combinator_grounds_its_declared_outputs`
  (`src/check/combinators.rs`). Part 1's output grounding is proven non-placebo
  by mutation (deleting it re-breaks golden 6). The Part-3 actual-mint check has
  **no** integration mutation-test (see the mutation-test ruling in the test
  plan); it is witnessed only by
  `scrutinee_enum_id_of_family_none_when_type_looks_concrete_but_unminted`.
  Full green gate; exactly four flips plus one added test (golden 6b) and the
  slice12 fixture's orthogonal linearity fix; no other suite changes.
- **Effort / difficulty.** Medium effort; **hard** — the splice-site grounding,
  the scrutinee actual-mint guard (permissiveness is the trap), the shared
  `env`-miss fallback, and four coupled migrations must land atomically green.

## Phases (JSON)

```json
[
  {
    "phase": 1,
    "focus": "Id-indexed decl fallback infrastructure: struct_decl twin in src/ast.rs, closure-taking with_enum_decl_or_generic / with_struct_decl_or_generic in src/check/engine.rs, wired into the concrete-path id-indexed decl reads in src/check.rs (src/check/poly.rs is NOT modified; its body-walk reads are the standalone path already covered, and the generics header table must not be touched). No behavior change; tree stays green with zero golden flips (golden 10 does not panic at HEAD). Unit tests for struct_decl and the Ctx accessors.",
    "effort": "small-medium",
    "difficulty": "medium"
  },
  {
    "phase": 2,
    "focus": "Check-time mint resolution: splice-site sig.outputs grounding in inline_combinator (src/check/combinators.rs, propagate apply_subst's Err), scrutinee_enum_id_of_family with a defensive actual-mint guard and mint_fallback_candidates (extended-slice + skip id derivation) at the env.get miss (src/check/terms.rs). Migrate the four bug-pinning fixtures atomically (three flips in tests/phase7_slice11.rs, one flip plus an independent Nil-arm linearity fix in tests/phase7_slice12.rs) and add the new golden 6b. Golden 6 and the slice12 fixture assert stdout. Unit tests for each new guard; Part 1's grounding is mutation-tested against golden 6, the Part-3 guard has no integration witness (unit-test only).",
    "effort": "medium",
    "difficulty": "hard"
  }
]
```

## Acceptance criteria

1. Goldens 6, 9, 10 (`tests/phase7_slice11.rs`) and slice12's migrated fixture
   flip to `build_and_run` and run to exit 0, with updated doc comments; the new
   golden 6b builds and runs to exit 0; golden 6 and the slice12 fixture assert
   their stdout value, not just exit code. Golden 8 stays an existing, unrelated
   always-green accept test (not a flip). Golden 7, goldens 4/5, and the three
   slice12 rejection tests
   (`a_generic_eliminator_in_a_concrete_body_is_rejected`, the standalone twin,
   and the R15 slot test) stay `build_error`, unchanged.
2. Each new guard has beside-it unit coverage. Part 1's grounding is proven
   non-placebo by mutation against golden 6; the Part-3 actual-mint check is
   defensive coding with no integration witness (unit-test only), and the false
   "deleting it re-breaks slice12" mutation claim is not asserted.
3. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green,
   with exactly the four documented golden flips plus one added test (golden 6b)
   and the slice12 fixture's orthogonal `Nil`-arm linearity fix, and no other
   suite changes.
4. The fabricated-instantiation regression guard
   (`concrete_body_generic_eliminator_message_does_not_fabricate_an_instantiation`'s
   negative assertions) is retired as a stated, accepted tradeoff, not silently
   dropped.
