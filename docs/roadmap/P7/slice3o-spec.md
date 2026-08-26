# Spec: P7.S3o — Bound dispatch on a poly combinator's own type variable

**Status:** Draft  
**Created:** 2026-08-26  
**Discovery:** docs/roadmap/P7/slice3o-brief.md

## Problem Statement

A combinator (an `inline` word) is spliced at its call sites — no call frame,
no indirection. When such a combinator carries a `'T: TraitName` bound on its
own type variable and its body calls a bare trait member (`cmp` directly, not
the exported `gt` wrapper), the bound cannot dispatch: `reject_user_bound_on_
combinator` rejects it at the gate before the body is ever checked. The
fallback is to ship the combinator non-inline, paying a real call frame per
instantiation — the shape S3s deliberately chose for `mymax`/`mymax3` to
avoid this slice.

Behind that rejection lie three independent gaps: (1) a pre-existing
span-keyed `insts` collision that silently miscompiles any poly combinator
calling a poly word at two types — the real blocker, now guarded by the
committed 1a overwrite-detector (507e0b7) which converts the miscompile into a
located error; (2) the splice path (`check_terms_relaxed`) has zero
bound-dispatch calls — `poly_trait_member_call` is never invoked there, so
bare members read as "unknown word"; (3) nothing records an obligation for a
combinator's body, so the resolution tables are empty. This slice closes all
three gaps, rejects the materialized-quotation corner (option b, settled by
probe 2), and validates the fix by differential-testing the inline motivating
program (`mymax`/`mymax3`) against the non-inline baseline at two splices and
two types.

## Requirements

- **R1.** A poly combinator (an `inline` word) whose body calls a polymorphic
  word, spliced at two or more distinct concrete types, must compile and run
  correctly — each splice's inner-call instantiation must be monomorphized
  independently, dispatching to the correct monomorph for each concrete type.
- **R2.** The per-splice instantiation mechanism must derive inner-call
  instantiations from the splice's θ (already computed by
  `check_poly_combinator_args`) on the check side, reusing `check_poly_call`'s
  existing unification + `instantiation_symbol` to mint a per-splice `CallInst`
  per inner poly call. This introduces no new check/lower key-consistency
  invariant beyond the existing `inline_uid` threading.
- **R3.** A bounded poly combinator (`'T: TraitName`) whose body calls a bare
  trait member (the member name directly, not a wrapper word) must resolve the
  member to the correct `impl:` at each concrete splice site, dispatching to
  the right implementation for each concrete type.
- **R4.** The bound-dispatch resolution at the splice site must be transitive
  over the splice tree: an unbounded combinator whose body splices a bounded
  one must not trigger a rejection for the inner combinator's bound member
  call, neither at the outer combinator's standalone check nor at the splice
  site.
- **R5.** Bound dispatch inside a materialized quotation within a bounded
  combinator must be rejected with a located error rather than silently
  miscompiled.
- **R6.** The motivating program `examples/poly_if.sth` with `mymax` and
  `mymax3` restored to `inline` with `'T: Copy Ord` must produce byte-identical
  stdout to the non-inline baseline when run at two concrete types (i64 and
  f64).
- **R7.** The oracle harness must validate the inline build against the
  non-inline baseline at two splices of the enclosing combinator (`mymax3`)
  and two concrete types, diffing both program stdout and resolved `impl:`
  dispatch targets via the `nm`/`objdump` call-graph walk already in the
  harness skeleton.
- **R8.** `tests/corpus_stdout/poly_if.txt` must remain byte-identical after
  `mymax`/`mymax3` are flipped to `inline`.
- **R9.** The fix must not change the compilation behavior of any existing
  program that does not splice a poly combinator calling a poly word at two
  types — the existing test corpus and golden tests must pass unchanged.
- **R10.** Every design shape in this spec must be validated with two splices
  of the enclosing combinator at two concrete types where one type matches the
  `i64` stand-in used by the standalone check
  (`check_poly_combinator_standalone`).

## Success Criteria

- [ ] An unbounded inline combinator calling a poly word, spliced at i64 and
      f64, compiles and runs correctly — two distinct monomorphs emitted, each
      dispatching to the right callee (R1).
- [ ] The `pid`/`c` fixture (currently hits the 1a guard) compiles and runs
      correctly instead of erroring (R1).
- [ ] A bounded inline combinator calling a bare trait member (`cmp`
      directly) resolves to the correct `impl:` at i64 and f64 — two distinct
      monomorphs, correct output (R3).
- [ ] An unbounded combinator splicing a bounded one does not reject the
      inner combinator's bound member call (R4).
- [ ] Bound dispatch inside a materialized quotation from a bounded
      combinator produces a located error (R5).
- [ ] `mymax`/`mymax3` inline with `'T: Copy Ord` produce byte-identical stdout
      to the non-inline baseline (R6, R8).
- [ ] The oracle harness diffs the inline build against the non-inline
      baseline on both stdout and dispatch targets, at two splices and two
      types (R7).
- [ ] `cargo fmt --check && cargo clippy -- -D warnings && cargo test` passes
      with no regressions (R9).
- [ ] Every new golden test exercises two splices at two types, one matching
      `i64` (R10).

## Scope & Boundaries

**In scope:**

- Item 1c: per-splice instantiation records via splice log + check-side
  derivation (the hybrid of shape iii from the brief, refined by round-4
  probe 3). The checker logs each splice's θ and derives the inner-call
  `CallInst`s from that θ on the check side — redirecting `check_poly_call`'s
  existing `CallInst` mint — not on the lowering side.
- Item 3: dispatch injection into `check_terms_relaxed` — threading
  `poly_trait_member_call` plus the `sig`/`TraitCtx` needed to resolve a bare
  trait member at the splice site. The transitive skip via a pre-pass body
  scan of the combinator's own `WordDef` (not a `Provenance` flag).
- Option (b): reject bound dispatch inside materialized quotations from
  bounded combinators with a located error and a test.
- Oracle harness: flip `mymax`/`mymax3` to inline, add a fixture calling both
  words at two types, diff against the non-inline baseline.
- Removal of `reject_user_bound_on_combinator`'s call site (the gate that
  currently rejects all `Bound::User` on a combinator's own type variable).

**Out of scope:**

- Re-specifying the 1a overwrite-detector guard (already committed as
  507e0b7; stays as a safety net).
- Modifying `poly_trait_member_call`, `resolve_user_bound`, `CallInst`, or
  the non-combinator `trait_calls` mechanism (P7.S3e) — these are reused as-is.
- Ambiguity/disambiguation rules for two bounds sharing a member name
  (P7.S3p's rulings inherited as-is).
- A hand-written native backend or WASM lowering (CLAUDE.md load-bearing
  invariants).
- In-process JIT or comptime interpreter (out of scope per CLAUDE.md).

## Solution Approach

The spec adopts a **hybrid of shape (iii)** for item 1c, refined by the
round-4 probes. The checker logs each splice's θ (already computed by
`check_poly_combinator_args` and currently discarded) and derives the
inner-call `CallInst`s **on the check side**, not the driver side. This avoids
the `pub(super)` visibility barrier (`check_poly_call` and `unify_poly_input`
are `pub(super)`, inaccessible from `src/ir/driver.rs`) and reuses the existing
`discover_transitive_instantiations` fixpoint (`src/check/poly.rs:4940`) for
transitive discovery: the derived per-splice `CallInst`s feed the fixpoint
naturally, preserving poly chains (combinator → p1 → p2). The probe confirmed
the existing `poly_cross_calls`/`compose` mechanism cannot be used as-is
(combinators are skipped in the pre-pass, the splice path uses `insts` not
`poly_cross_calls`, and `compose` rejects combinator callees), but the
fixpoint's seeding logic is reusable once that `compose` combinator-callee
rejection is relaxed/bypassed for splice-derived records (P1-2: a
splice-provenance check in `compose`, or a new seeding entry point that
bypasses the cross-call walk) — not merely by inserting into the span-keyed
`module.instantiations`, which would re-introduce the 1a collision (P1-3).

The approach adds no new check/lower key-consistency invariant: the
`inline_uid` already threads both sides (`src/check/combinators.rs:506` /
`src/ir/func_builder/calls.rs:638`), and resolution happens where θ is actually
known rather than where spans collide. The 1a overwrite-detector guard
(507e0b7) stays as a safety net but is no longer triggered for
spliced-combinator inner calls because the checker skips the span-keyed
`insts.insert` for those calls. Removing `reject_user_bound_on_combinator`'s
call site at `src/check.rs:895` unblocks bounded combinators: the standalone
check (`check_poly_combinator_standalone`) already substitutes `i64` and
checks bounds against the real `TraitResolveCtx` (line 894), so a body calling
a real poly word (`gt`) passes; a body calling a bare member (`cmp`) fails with
a legible "unknown word" error until item 3 lands.

**Bounded inner words — two cases (probe 4 finding).** The probe discovered
that "bounded inner word" splits into two cases with different check-time
behavior:

- **Case (a): bounded poly word** (e.g. `gt`, a regular poly word with
  `'T: Ord` that internally calls `cmp`). `trait_calls` IS seeded at check
  time by `resolve_user_bound` inside `check_poly_call` at each splice site.
  Phase 1 handles this case: the per-splice records carry the seeded
  `trait_calls` map, and lowering dispatches correctly. The motivating program
  (`poly_if.sth`) calls `gt`, not `cmp` directly, so Phase 1 + Phase 2 suffice
  for it.
- **Case (b): bare member call** (e.g. `cmp` called directly from a
  combinator body). `trait_calls` is NOT seeded — `check_terms_relaxed` lacks
  `poly_trait_member_call` (Gap 2) and no obligation is recorded (Gap 3). This
  case genuinely needs Phase 3's dispatch injection. It is the hot-path
  optimization (calling `cmp` directly instead of through `gt`).

For item 3, the spec threads `poly_trait_member_call` and the `sig`
(`PolySig` carrying the bounds) plus the `TraitCtx`/`TraitResolveCtx` into
`check_terms_relaxed` — the splice path that currently has zero
bound-dispatch calls. At the splice site, θ is concrete (from
`check_poly_combinator_args`), so a bare member call resolves to the correct
`impl:` per splice, reusing `resolve_user_bound`'s `impl:`-registry lookup.
The transitive skip is a **pre-pass body scan**, not a runtime `Provenance`
flag: the standalone check (`check_poly_combinator_standalone`) runs in the
pre-pass (`src/check.rs:920`) with no splice active, so a flag set during
splicing would never be set when the standalone check runs. Instead, before
the i64 body walk, the standalone check scans the combinator's own `WordDef`
for calls to bare trait members (names matching a `Bound::User` member in the
combinator's `poly` sig) and skips the i64 walk for those terms — they are
checked at the splice site where θ is concrete. Nested combinators are
handled naturally: each combinator's standalone check skips its own member
calls, and the splice walk resolves them at the concrete splice θ. The
resolved `impl:` symbol is delivered to lowering via the per-splice mechanism
from item 1c.

For option (b), the spec rejects bound dispatch inside materialized
quotations from bounded combinators with a located error. Probe 2 confirmed
this is sound: the motivating program's `~[ ]` arms are spliced by
`branch`/`lower_if` into basic blocks (never materialized — zero `__quot`
symbols), and the case the rejection targets is currently unconstructible
(three independent gates block it) but correct to reject when it becomes
constructible, because the materialized quotation gets its own `IrFunc` with
no splice-site prefix and two splices would collide.

The oracle harness (`tests/phase7_slice3s_oracle.rs`) already builds
`examples/poly_if.sth` and diffs against itself, proving the plumbing (build,
run, `nm`/`objdump` call-graph walk) works. S3o gives it a real second variant:
a non-inline baseline (the current source with `inline` removed) vs. the
inline candidate (same source with `inline` restored). A new fixture calling
both `mymax` and `mymax3` at two types is needed because `main` currently calls
`mymax3` only — `mymax` mints no monomorph and the harness never sees it.

## Codebase Map

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/check/poly.rs:4576` | `check_poly_call` | The function that records inner-call `CallInst`s via `insts.insert` at line ~4885. Phase 1 skips the insert for spliced-combinator inner calls; the 1a guard at line ~4878 stays as a safety net. The check-side derivation reuses `check_poly_call`'s unification + `instantiation_symbol` (accessible from within `check/`). |
| `src/check/poly.rs:4878` | 1a overwrite-detector guard | Committed as 507e0b7. Stays as-is; no longer triggered for spliced-combinator inner calls after phase 1 skips the insert. |
| `src/check/poly.rs:4361` | `splice_collision_error` | The 1a guard's error message. No change; stays as safety net. |
| `src/check/poly.rs:6163` | `reject_user_bound_on_combinator` | The gate rejecting `Bound::User` on a combinator's own type variable. Phase 1 removes its call site at `src/check.rs:895`; the function (and `user_bound_on_combinator_error` at `src/check/poly.rs:6187`) becomes dead code and must be `#[allow(dead_code)]`-annotated or removed (P1-7). |
| `src/check/poly.rs:361` | `check_poly_combinator_standalone` | The `i64` stand-in check. Phase 3 skips it for member-dispatching bodies (Gap 1). Uses a scratch `PolyCtx` (already isolated per 1b). |
| `src/check/poly.rs:908` | `poly_trait_member_call` | The bound-dispatch machinery (operand check + obligation recording). Called only in `poly_call_term` at line ~1135; phase 3 threads it into `check_terms_relaxed`. |
| `src/check/poly.rs:5313` | `resolve_user_bound` | The per-call-site `impl:`-registry lookup + symbol resolution, inserting into a `trait_calls: HashMap<Span, String>`. Phase 3 reuses it at the splice site. |
| `src/check/poly.rs:107` | `TraitResolveCtx` | The tables for bound resolution (`traits`, `impls`, `word_symbols`, `recorded`). Already real for the standalone check; phase 3 threads it into the splice path. |
| `src/check/poly.rs:26` | `TraitCtx` | The poly-body walk's trait context (`traits`, `obligations`). Phase 3 threads it into `check_terms_relaxed`. |
| `src/check/combinators.rs:347` | `inline_combinator` | The splice entry point. Captures `poly_subst` (θ) at line 358 from `check_poly_combinator_args`; phase 1 logs it, phase 3 threads dispatch. |
| `src/check/combinators.rs:571` | `check_poly_combinator_args` | Computes θ (`Subst`) at the splice site; returns it at line 683 (`Ok(subst)`). θ is currently used for the back-edge shape only; phase 1 logs it for lowering. |
| `src/check/combinators.rs:506` | `inline_uid` increment | The splice-site uid counter (`prov.inline_uid`). Already threads both sides (checker line 506 / lowering line 638). Phase 3 sets/clears the transitive skip flag here. |
| `src/check/combinators.rs:527` | `check_terms_relaxed` call | Where the spliced body is re-walked with the real `PolyCtx`. Phase 3 threads dispatch machinery into this call. |
| `src/check.rs:895` | `is_combinator` pre-pass arm | Calls `reject_user_bound_on_combinator` before the standalone check. Phase 1 removes the call; the scratch `PolyCtx` (line 898) and standalone check proceed. |
| `src/check/terms.rs:53` | `check_terms_relaxed` | The splice path with zero bound-dispatch calls. Phase 3 injects `poly_trait_member_call` dispatch here. Calls `check_poly_call` at line 770 for poly word calls. |
| `src/ir/func_builder/calls.rs:344` | `instantiations` lookup | `self.instantiations.get(&span)` — the span-keyed lookup that collides. Phase 1 adds per-splice override for spliced-combinator inner calls. |
| `src/ir/func_builder/calls.rs:638` | `lower_call` combinator splice | The lowering-side splice: increments `inline_uid` (line 638-639), alpha-renames, and calls `lower_terms`. Phase 1 threads per-splice instantiations; phase 3 delivers resolved `impl:` symbols. |
| `src/ir/func_builder/mod.rs:193` | `FuncBuilder` | The lowering builder. `inline_uid: u32` at line 367; `instantiations`, `trait_calls`, `poly_calls` fields. Phase 1 adds per-splice instantiation access. |
| `src/ir/func_builder/mod.rs:1061` | `lower_materialized` | Materialized quotation lowering: mints a fresh `FuncBuilder` with `inline_uid: 0`. Phase 4 rejection context. |
| `src/ir/driver.rs:262` | monomorphization loop | Dedups by symbol, lowers one `IrFunc` per `(callee, θ)`. Phase 1 reads pre-derived per-splice `CallInst`s from `module.splice_records` (no type-stack walk in the driver). |
| `src/ir/driver.rs:669` | `concrete_effect` | Applies θ to a `PolySig`'s inputs/outputs to produce concrete `StackEffect`. Phase 1 uses it to derive inner-call instantiations from the splice θ. |
| `src/ast.rs:18` | `Module` | The module struct. `instantiations` at line 64, `transitive_instantiations` at line 79. Phase 1 adds a `splice_records` field. |
| `src/ast.rs:1987` | `CallInst` | The instantiation record (`callee`, `subst`, `symbol`, `trait_calls`, `poly_calls`). Reused as-is; phase 1 creates per-splice instances. |
| `src/check/engine.rs:179` | `Provenance` | Threaded checker state. `self_tail_combinator` at line 179, `inline_uid: u32` at line 186. Phase 1 may add an `in_splice` flag for `check_poly_call`. Phase 3's transitive skip is a pre-pass body scan, not a Provenance flag. |
| `src/check/audits.rs:463` | `reject_poly_quotation_anywhere` | Audit gate for quotation types with type variables as combinator outputs. Phase 4 rejection context (one of three gates). |
| `tests/phase7_slice3s_oracle.rs` | oracle harness skeleton | Builds `examples/poly_if.sth`, diffs stdout + dispatch targets against itself. Phase 2 modifies it to build two variants (non-inline baseline vs. inline candidate). |
| `tests/phase4_combinators.rs` | `check_splice_collision_two_types_is_error` | The 1a guard test (`pid`/`c` fixture). Phase 1 updates it: the fixture now compiles correctly instead of erroring. |
| `examples/poly_if.sth` | motivating program | `mymax` (line 18) and `mymax3` (line 24) are currently non-inline. Phase 2 restores `inline`. `main` calls `mymax3` only (lines 28-30). |
| `tests/corpus_stdout/poly_if.txt` | corpus golden | `9\n9\n` — must stay byte-identical (R8). No change to this file. |
| `lib/combinators.sth` | `map`/`each`/`fold` combinators | Hot-path use cases: `map inline ( ['T 'N] ~[ 'T -- 'T ] -- ['T 'N] )` at line 48. These call quotation parameters, not poly words, so they are unaffected. |

## Open Questions

- [x] ~~Exactly how lowering derives inner-call instantiations from θ~~ —
  Resolved by round-4 probe 3: derivation stays on the **check side** (not
  the driver), avoiding the `pub(super)` visibility barrier. The checker uses
  the splice θ directly (already computed by `check_poly_combinator_args`) to
  mint per-splice `CallInst`s, and feeds them into the existing
  `discover_transitive_instantiations` fixpoint for transitive discovery. No
  driver-side type-stack walk is needed.
- [x] ~~How the resolved `impl:` symbol for a bare member call is delivered
  to lowering~~ — Resolved by round-4 probe 4: "bounded inner word" splits
  into two cases. Case (a) — bounded poly words like `gt`: `trait_calls` IS
  seeded at check time by `resolve_user_bound` inside `check_poly_call` at
  each splice site, so Phase 1's per-splice records carry it naturally. Case
  (b) — bare member calls like `cmp`: `trait_calls` is NOT seeded (Gap 2 + 3);
  Phase 3's dispatch injection seeds it, and the resolved `impl:` is
  delivered via the per-splice `CallInst`'s `trait_calls` map from Phase 1.
- [x] ~~Whether option (b) blocks the motivating program~~ — Resolved by
  probe 2: the motivating program's `~[ ]` arms are spliced, not materialized
  (zero `__quot` symbols). Option (b) is sound and does not block.
- [x] ~~Whether the `i64` stand-in check's `insts` are a writer~~ — Resolved
  by probe 1: the build path already isolates the stand-in's `insts` into a
  scratch `HashMap` (`src/check.rs:898`). The collision is purely between real
  splices.
- [x] ~~Whether the materialized-quotation corner is constructible~~ —
  Resolved by probe 1d: three independent gates block it. The corner is
  gated-but-watched, not a blocker.

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| The check-side derivation of inner-call instantiations from θ is incorrect or incomplete (the brief says all shapes need the two-splice, two-type oracle) | Med | The oracle harness (phase 2) and the two-splice/two-type testing discipline (R10) validate every shape. The `pid`/`c` fixture is the minimal two-type probe. |
| Removing `reject_user_bound_on_combinator` exposes new failure modes for bounded combinators calling bare members (Gap 1: standalone check fails with "unknown word `cmp`") | Med | The standalone check catches bare member calls with a legible (if misleading) error until phase 3 fixes them. No existing program is affected (the rejection prevented them). |
| The transitive skip mechanism is incorrect (round 2 found the per-combinator skip breaks the inner splice's rewrite) | Low | The transitive skip is a pre-pass body scan (not a runtime flag), scoped to the combinator's own `WordDef`. Nested combinators are handled naturally: each combinator's standalone check skips its own member calls, and the splice walk resolves them at the concrete splice θ. The two-splice discipline validates it at depth two (probe 3 confirmed stability). |
| The materialized-quotation rejection fires for the motivating program (whose `~[ ]` arms are spliced, not materialized) | Low | Probe 2 confirmed zero `__quot` symbols for the motivating program. The rejection targets only materialized quotations, which are triggered by escaping quotations, not inline combinator arms. |
| The per-splice instantiation mechanism changes the compilation of existing programs | Low | R9 (no regression) and the existing test corpus. The mechanism only affects spliced-combinator inner calls; existing combinators call builtins/intrinsics/quotation parameters, not plain poly words. |
| Bounded combinators calling bare members ICE between Phase 1 and Phase 3 | Med | Probe 4 split this into two cases. Case (a) — bounded poly words like `gt`: `trait_calls` is seeded at check time, Phase 1 handles it. Case (b) — bare member calls like `cmp`: needs Phase 3. The intermediate-state test pins case (b) as a legible error ("unknown word"), not an ICE. |

## Delivery Plan

### Phase 1: Per-splice instantiation records (item 1c, hybrid shape iii)

- **Goal**: An inline combinator calling a polymorphic word (unbounded or
  bounded poly word like `gt`), spliced at two distinct concrete types, compiles
  and runs correctly — the `pid`/`c` fixture prints correct values instead of
  hitting the 1a guard's collision error, and a combinator calling `gt` at two
  types dispatches to the correct `impl:` for each type.
- **Requirements Covered**: R1, R2, R9, R10
- **Scope**:
  - `src/check.rs:895` (`reject_user_bound_on_combinator` call) — remove the
    call so bounded combinators proceed to the standalone check. Only the
    call site is removed. The scratch `PolyCtx` (line 898) and standalone
    check proceed unchanged.
  - **Dead-code handling (P1-7)**: with the call site at `src/check.rs:895`
    removed, `reject_user_bound_on_combinator`
    (`src/check/poly.rs:6163`) and `user_bound_on_combinator_error`
    (`src/check/poly.rs:6187`) become dead code, which fails
    `cargo clippy -D warnings`. No test *code* invokes either function — the
    only `tests/` matches are doc-comment references
    (`tests/phase4_generics.rs:1114`, `tests/phase7_slice3s_oracle.rs:3`,
    `tests/phase4_slice10c_primitives.rs:251` and `:362`) that name the
    function in explanatory text, not call it. Both remedies are viable:
    annotate both functions with `#[allow(dead_code)]` (simpler — preserves
    the documented rationale and leaves the test doc-comment references
    non-stale) or remove both functions entirely (no test code depends on
    them, but the doc-comment references would go stale). The implementer
    should pick the simpler option; `#[allow(dead_code)]` is recommended.
  - `src/check/combinators.rs:347` (`inline_combinator`) — after
    `check_poly_combinator_args` returns θ (captured as `poly_subst` at line
    358), log a splice record `(caller_word, inline_uid, comb_name, θ)` to the
    module's splice log. The `inline_uid` is minted at line 506.
  - `src/check/combinators.rs:571` (`check_poly_combinator_args`) — θ is
    already returned at line 683 (`Ok(subst)`). No change to the function
    itself; the caller (`inline_combinator`) logs the result.
  - `src/check/poly.rs:4576` (`check_poly_call`) — the `insts.insert` at line
    ~4885. For inner poly calls encountered during a spliced-combinator body
    walk, skip the `insts.insert` (and the 1a guard at line ~4878 is never
    reached for these calls). The instantiation is recorded in the per-splice
    record instead (see `splice_records` below). Detection of "inside a splice"
    may use a `Provenance` flag (set at `src/check/combinators.rs:506`,
    threaded into `check_poly_call` via `poly`).
  - **Check-side derivation (hybrid approach, probe 3 finding)**: Instead of a
    driver-side type-stack walk, the checker derives per-splice `CallInst`s
    directly from the splice θ during `inline_combinator` (after
    `check_poly_combinator_args` returns θ). This avoids the `pub(super)`
    visibility barrier (`check_poly_call` and `unify_poly_input` are
    `pub(super)`, inaccessible from `src/ir/driver.rs`). The derivation reuses
    `check_poly_call`'s unification + `instantiation_symbol` logic (accessible
    from within `check/`) to mint a `CallInst` per inner poly call, keyed by
    `(inline_uid, body_span)` — unique within each splice.
  - **Derivation shape (P1-1)**: the derivation IS the redirect, not a
    separate post-walk pass. `check_poly_call` already mints a `CallInst`
    (with `trait_calls` for case (a) bounded poly words) during the
    `check_terms_relaxed` body walk — it is the same unification +
    `instantiation_symbol` mint that already runs for every poly call. Phase
    1 does not re-walk the body to mint a second `CallInst`; it captures the
    already-minted one into a per-splice record (`splice_records`) instead
    of `insts.insert`. An `in_splice` `Provenance` flag (or equivalent), set
    at `src/check/combinators.rs:506` and threaded into `check_poly_call`
    via `poly`, tells `check_poly_call` to redirect the already-minted
    `CallInst` to `splice_records` instead of `insts`. There is no separate
    post-walk derivation pass.
  - **Bounded poly words (probe 4 finding, case a)**: When the inner poly word
    is a bounded word like `gt` (a regular poly word with `'T: Ord` that
    internally calls `cmp`), `resolve_user_bound` inside `check_poly_call`
    already seeds `trait_calls` at each splice site. The per-splice record
    carries this seeded `trait_calls` map, so lowering dispatches correctly
    without Phase 3. This is the case the motivating program (`poly_if.sth`)
    exercises — `mymax`/`mymax3` call `gt`, not `cmp` directly.
  - **Transitive discovery preservation (P1-2)**:
    `discover_transitive_instantiations` (`src/check/poly.rs:4940`) seeds
    from `insts.values()` (the span-keyed `module.instantiations`, populated
    from `poly.insts` during check finalization), and its `compose` helper
    (`cross_calls_of`, the `inline_callee_cross_call_error` path at
    `src/check/poly.rs:5137`) explicitly rejects combinator callees.
    Skipping `insts.insert` for spliced-combinator inner calls means those
    calls are invisible to the fixpoint — a poly chain (combinator → p1 →
    p2) would fail to discover p2, causing an ICE at lowering. Naively
    inserting the derived `CallInst`s into the span-keyed
    `module.instantiations` is not enough and re-introduces the 1a
    collision (P1-3): the `compose` helper would still reject the
    combinator callee during cross-call composition. The rejection must be
    relaxed or bypassed for splice-derived records — either (a) the
    `compose` helper checks for a splice provenance on the `CallInst` and
    skips the combinator-callee rejection, or (b) splice-derived `CallInst`s
    seed the fixpoint directly via a new entry point that bypasses the
    `compose` cross-call walk. The exact mechanism is left to the
    implementer; the binding constraint is that a three-word chain
    (combinator → p1 → p2) at two types must compile and run correctly
    before the phase exits (exit criterion).
  - `src/ast.rs:18` (`Module`) — add a `splice_records` field (e.g.,
    `Vec<SpliceRecord>` or `HashMap<u32, SpliceRecord>`) to hold the splice
    log. Each `SpliceRecord` carries `(inline_uid, comb_name, θ, CallInsts)`
    where `CallInsts` is the set of per-splice derived `CallInst`s (including
    their `trait_calls` maps for case (a) bounded poly words). Model the new
    struct on existing `Module` fields like `transitive_instantiations`
    (line 79).
  - **Data flow — two consumers, two access paths (P1-3)**: the derived
    per-splice `CallInst`s have two consumers that must not share one
    table. `splice_records` (keyed by `(inline_uid, body_span)`) is the
    *lowering's* lookup path — `src/ir/func_builder/calls.rs:344` reads
    per-splice instantiations from it instead of the colliding span-keyed
    `instantiations` table. The fixpoint
    (`discover_transitive_instantiations`) reaches the *same* derived
    records via the P1-2 mechanism (a splice-provenance check in `compose`
    or a new seeding entry point) — not by naively inserting into the
    span-keyed `module.instantiations`, which would re-introduce the 1a
    collision. The two consumers use different access paths to the same
    derived records: `splice_records` is the single source of truth for
    lowering, and the fixpoint sees the records through the
    relaxed/bypassed `compose` path.
  - `src/ir/driver.rs:262` (monomorphization loop) — the driver's role is
    simplified: it reads the pre-derived per-splice `CallInst`s from
    `module.splice_records` (already minted by the checker) and enqueues them
    for monomorphization. No type-stack walk or unification logic in the
    driver — the checker has already done the derivation. The driver dedups
    by symbol as before.
  - `src/ir/func_builder/calls.rs:344` (`instantiations` lookup) — for inner
    poly calls inside a spliced combinator body, use the per-splice derived
    instantiations instead of the span-keyed `instantiations` table. The
    `FuncBuilder` knows its current `inline_uid` (line 367 in
    `src/ir/func_builder/mod.rs`); the per-splice instantiations are keyed by
    `inline_uid` + body span (unique within one splice).
  - `src/ir/func_builder/calls.rs:638` (`lower_call` combinator splice) —
    thread the per-splice instantiations into the splice so inner poly calls
    resolve correctly. The `inline_uid` increment (line 638-639) already
    matches the checker's (line 506). For case (a) bounded poly words, the
    per-splice `trait_calls` map is also threaded, so lowering dispatches to
    the correct `impl:`.
  - **inline_uid nesting (P1-4)**: the `inline_uid` counter is monotonic
    with no save/restore at either splice site — checker
    (`src/check/combinators.rs:506`: `let uid = prov.inline_uid;
    prov.inline_uid += 1;`) and lowering
    (`src/ir/func_builder/calls.rs:638`: `let uid = self.inline_uid;
    self.inline_uid += 1;`). For nested combinators (a combinator splicing a
    combinator), nested splices produce nested `inline_uid` values that do
    not unwind, so the lowering-side per-splice lookup cannot rely on a
    single "current `inline_uid`": it must use a stack of active
    `inline_uid`s (push on splice entry, pop on splice exit) keyed against
    `splice_records`. Phase 1's tests do not exercise nesting (the nested
    case is Phase 3's R4), but `splice_records` (keyed by `inline_uid`)
    must be designed to support the stack-of-active-uids lookup from the
    start.
  - `tests/phase4_combinators.rs` (`check_splice_collision_two_types_is_error`)
    — update this test: the `pid`/`c` fixture should now compile and run
    correctly at two types instead of producing a collision error. Replace
    the error assertion with a correct-compilation golden test.
  - `tests/phase7_slice3e.rs:512` (`a_user_bound_on_a_poly_combinator_is_rejected`)
    — update this test: it currently asserts the gate rejection message that
    Phase 1 removes. After the gate call at `src/check.rs:895` is removed, the
    `shows` combinator (with `'T: Show`, a bare member) proceeds to the
    standalone check, where `show` is a bare member (case (b)) not yet
    dispatchable — it falls through the standalone check's `env.get`
    fallthrough (`unknown_word_error` at `src/check/terms.rs:795`) and
    produces a legible "unknown word `show`" error, NOT a successful compile.
    The test's assertion must change from the old gate rejection message
    (`` `'T: Show` on the combinator `shows` ... is not supported``) to the
    NEW "unknown word `show`" error from the standalone check. The gate
    rejection is gone; the standalone check catches the bare member with a
    legible error. (This is the intermediate-state pin from the risks table:
    case (b) is a legible "unknown word" error, not an ICE, until Phase 3.)
  - New test file (e.g., `tests/phase7_slice3o.rs` or added to
    `tests/phase4_combinators.rs`): a golden test with two splices of a poly
    combinator calling a poly word at i64 and f64, asserting two distinct
    monomorphs are emitted and the program output is correct.
  - New test: a bounded poly word (`gt`) called from an inline combinator at
    two types (i64 and f64), asserting correct dispatch to the right `impl:`
    for each type (case (a), probe 4 finding). Two distinct monomorphs are
    emitted. This is the shape the motivating program exercises.
  - New test: a three-word chain (combinator → p1 → p2) at two types,
    asserting transitive discovery is preserved (P1-2 fix).
- **Explicitly out of scope for this phase**:
  - Dispatch injection for bare trait members (item 3) — phase 3.
  - Rejecting materialized quotations (option b) — phase 4.
  - Oracle harness modification and flipping `mymax`/`mymax3` — phase 2.
  - Modifying `poly_trait_member_call`, `resolve_user_bound`, or `CallInst` —
    reused as-is.
- **Entry Conditions**: The 1a overwrite-detector guard is committed on main
  (507e0b7). The scratch `PolyCtx` isolation (1b) is already in place
  (`src/check.rs:898`).
- **Exit Criteria / Verifiable Artifacts**:
  - The `pid`/`c` fixture compiles and runs correctly at two types (i64 and
    f64), printing correct values — no collision error.
  - A two-splice, two-type golden test passes: `nm` on the built binary shows
    two distinct monomorphs for the inner poly call (one per type).
  - A bounded poly word (`gt`) called from an inline combinator at two types
    (i64 and f64) compiles and dispatches to the correct `impl:` for each
    type — two distinct monomorphs, correct output (case (a)).
  - A three-word chain test (combinator → p1 → p2) at two types compiles and
    runs correctly — transitive discovery is preserved (P1-2 fix).
  - The updated `check_splice_collision_two_types_is_error` test asserts
    correct compilation, not an error.
  - The updated `a_user_bound_on_a_poly_combinator_is_rejected` test asserts
    the new "unknown word `show`" error from the standalone check (not the
    old gate rejection message).
  - An intermediate-state test pins the Phase 1→3 gap: a bounded combinator
    calling a bare member (`cmp` directly) produces a legible error ("unknown
    word" or similar), not an ICE.
  - `cargo fmt --check && cargo clippy -- -D warnings && cargo test` passes
    with no regressions (R9).
- **Parallelism**: SEQUENTIAL — must be first; all other phases depend on the
  per-splice instantiation mechanism and the removal of
  `reject_user_bound_on_combinator`.
- **Relative Effort**: M — the core fix, touching the checker-lowering
  interface and deriving per-splice `CallInst`s on the check side. Roughly a
  week.
- **Difficulty**: hard — cross-cutting refactor touching shared control flow
  between checker and lowering, with a new derivation mechanism that mirrors
  existing type-checking logic. The brief notes all three fix shapes need the
  two-splice, two-type oracle before any is trusted.
- **Open Questions / Blockers**: None remaining. Round-4 probes resolved
  both open questions: the derivation stays on the check side (probe 3),
  and bounded poly words (case a) are handled by carrying the check-time
  `trait_calls` map (probe 4). The implementer should validate with the
  `pid`/`c` fixture and the `gt`-from-combinator fixture at two types before
  trusting the approach.

### Phase 2: Oracle harness + motivating program flip

- **Goal**: `mymax` and `mymax3` restored to `inline` with `'T: Copy Ord`
  produce byte-identical stdout to the non-inline baseline, and the oracle
  harness diffs both stdout and dispatch targets at two splices and two types.
- **Requirements Covered**: R6, R7, R8
- **Scope**:
  - `examples/poly_if.sth` — restore `inline` on `mymax` (line 18) and
    `mymax3` (line 24). The `'T: Copy Ord` bounds stay. The `main` body
    (lines 28-30) stays unchanged — it already calls `mymax3` at i64 and f64.
  - `tests/phase7_slice3s_oracle.rs` — modify the harness to build two
    variants: the non-inline baseline (the current source with `inline`
    removed, or a generated copy) and the inline candidate
    (`examples/poly_if.sth` with `inline` restored). The harness skeleton's
    `build_run_and_dispatch_targets` function and the
    `poly_if_oracle_harness_reports_a_clean_diff_against_itself` test are the
    pattern to follow.
  - A new fixture source (e.g., `tests/fixtures/poly_if_both.sth` or a
    generated variant) that calls both `mymax` and `mymax3` at two types (i64
    and f64), since `main` calls `mymax3` only and `mymax` mints no monomorph.
    This exercises the "at two splices, at two types" diff the harness needs.
  - `tests/corpus_stdout/poly_if.txt` — must remain byte-identical (`9\n9\n`).
    No change to this file; verify the corpus test still passes after the
    inline flip.
- **Explicitly out of scope for this phase**:
  - Any compiler changes — phase 1 provides the fix.
  - Dispatch injection for bare trait members — phase 3.
  - Rejecting materialized quotations — phase 4.
- **Entry Conditions**: Phase 1 complete — the per-splice instantiation
  mechanism works, so inline `mymax`/`mymax3` at two types compile correctly
  without hitting the 1a guard.
- **Exit Criteria / Verifiable Artifacts**:
  - The oracle harness diffs the inline build against the non-inline
    baseline: stdout is byte-identical, and the resolved `impl:` dispatch
    targets (via `nm`/`objdump` call-graph walk) match at each `mymax*` splice
    site.
  - `tests/corpus_stdout/poly_if.txt` is byte-identical (the corpus test
    passes).
  - The fixture calls both `mymax` and `mymax3` at two types (i64 and f64),
    and the harness finds `mymax` monomorphs in the binary (it currently
    cannot because `main` calls `mymax3` only).
  - `cargo test` passes with no regressions.
- **Parallelism**: PARALLEL with phase 3 — both depend only on phase 1; they
  touch different code paths (test harness vs. checker dispatch).
- **Relative Effort**: S — test harness setup and a source flag flip; the
  harness skeleton is already in tree and the diff logic is already written.
- **Difficulty**: standard — test setup and a source modification.
- **Open Questions / Blockers**: The non-inline baseline source needs to be
  available to the harness. Options: keep a non-inline copy as a fixture file,
  or generate it at test time by stripping `inline` from the source. The
  implementer should choose the simpler option (a fixture file is more
  transparent).

### Phase 3: Dispatch injection into check_terms_relaxed (item 3)

- **Goal**: A bounded inline combinator calling a bare trait member (`cmp`
  directly) resolves the member to the correct `impl:` at each splice site,
  dispatching correctly at two concrete types (i64 and f64).
- **Requirements Covered**: R3, R4
- **Scope**:
  - `src/check/terms.rs:53` (`check_terms_relaxed`) — the splice path with
    zero bound-dispatch calls. Inject `poly_trait_member_call` (or equivalent
    dispatch logic) into the `TermKind::Call` arm: when a bare member name is
    encountered, check it against the combinator's bounds. Thread the
    `PolySig` (carrying the bounds) and the `TraitCtx`/`TraitResolveCtx`
    (carrying the `impl:` registry) needed to resolve the bound at the splice
    site, where θ is concrete.
  - `src/check/combinators.rs:347` (`inline_combinator`) — thread the
    combinator's `PolySig` and the `TraitResolveCtx` into the
    `check_terms_relaxed` call at line 527. The `PolySig` is available from
    `comb.word.poly` (checked at line 358). The `TraitResolveCtx` is available
    from `poly.trait_resolve`.
  - `src/check/combinators.rs:571` (`check_poly_combinator_args`) — θ is
    already computed here. Phase 3 threads it (or the obligation resolution
    derived from it) so the bare member call resolves at the splice site
    rather than being recorded as an abstract obligation.
  - `src/check/poly.rs:361` (`check_poly_combinator_standalone`) — the
    standalone check's i64 body walk cannot resolve bare trait members (Gap 1:
    the `i64` stand-in checker has zero trait-bound awareness, so a bare `cmp`
    fails a plain `env.get` lookup). The body is checked at the splice site
    instead, where θ is concrete and the bound resolves. **The skip must not
    use a runtime `Provenance` flag** — the standalone check runs in the
    pre-pass (`src/check.rs:920`) with no splice active, so a flag set during
    splicing would never be set when the standalone check runs. Instead, the
    skip is a **pre-pass scan** of the combinator's own body: before the i64
    body walk, scan the combinator's `WordDef` for calls to bare trait members
    (names matching a `Bound::User` member in the combinator's `poly` sig).
    If found, skip the i64 body walk for those terms — they will be checked at
    the splice site by Phase 3's dispatch injection. The scan is scoped to the
    combinator's own body (concrete `WordDef` with `poly: None` for the stand-
    in), not a threaded flag. Nested combinators (combinator splicing a
    bounded combinator) are handled naturally: the inner combinator's own
    standalone check runs in the same pre-pass and skips its own member
    calls; the outer combinator's splice walk reaches the inner combinator's
    splice, where Phase 3's dispatch injection resolves the member at the
    concrete splice θ.
  - **Nested combinator support (P1-4)**: explicit handling for a combinator
    splicing a bounded combinator (R4's depth-two case). The lowering-side
    per-splice lookup uses the stack of active `inline_uid`s designed in
    Phase 1 (P1-4) so the inner combinator's splice resolves its bare member
    call at the inner splice's concrete θ, not the outer's. The pre-pass
    body scan already scopes each combinator's own member calls; this bullet
    makes the lowering-side stack lookup explicit for the nested case. A
    nested two-type test (i64 + f64) at depth two is a Phase 3 exit
    criterion (in addition to the transitive-skip test below).
  - `src/check/poly.rs:908` (`poly_trait_member_call`) — reused as-is for the
    operand check and obligation identification. Called at the splice site
    (phase 3 adds the call in `check_terms_relaxed`), followed by
    `resolve_user_bound` to resolve the obligation against the concrete θ.
  - `src/check/poly.rs:5313` (`resolve_user_bound`) — reused as-is for the
    `impl:`-registry lookup at the splice site. The resolved symbol is
    delivered to lowering via the per-splice mechanism from phase 1 (the
    `CallInst`'s `trait_calls` map or equivalent per-splice channel).
  - `src/ir/func_builder/calls.rs:638` (`lower_call` splice) — deliver the
    resolved `impl:` symbol for bare member calls to lowering via the
    per-splice instantiations from phase 1. Lowering looks up the resolved
    symbol at the splice site (mirroring how it already reads
    `trait_calls` for non-combinator poly words).
  - New test: a bounded inline combinator calling `cmp` directly at two types
    (i64 and f64), asserting correct output and two distinct monomorphs
    (`nm` shows both `cmp;Ord;...;i64` and `cmp;Ord;...;f64` resolved).
  - New test: transitive skip — an unbounded inline combinator that splices a
    bounded one calling `cmp`, asserting no rejection and correct output.
- **Explicitly out of scope for this phase**:
  - The per-splice instantiation mechanism — phase 1.
  - The oracle harness — phase 2.
  - Rejecting materialized quotations — phase 4.
  - Modifying `poly_trait_member_call` or `resolve_user_bound` — reused as-is.
- **Entry Conditions**: Phase 1 complete — the per-splice instantiation
  mechanism provides the delivery channel for resolved `impl:` symbols, and
  `reject_user_bound_on_combinator`'s call site is already removed.
- **Exit Criteria / Verifiable Artifacts**:
  - A bounded inline combinator calling `cmp` directly compiles and runs
    correctly at two types (i64 and f64), dispatching to the correct `impl:`
    for each type. Two distinct monomorphs are emitted.
  - The transitive skip works: an unbounded combinator splicing a bounded one
    does not trigger a rejection for the inner combinator's bound member call.
  - Nested combinator support (P1-4): a combinator splicing a bounded
    combinator at depth two, at two types (i64 and f64), resolves the inner
    combinator's bare member call via the stack-of-active-`inline_uid`s lookup
    and dispatches correctly — no rejection, correct output.
  - Two-splice, two-type validation passes for the bare member case (R10).
  - `cargo fmt --check && cargo clippy -- -D warnings && cargo test` passes
    with no regressions.
- **Parallelism**: PARALLEL with phase 2 — both depend only on phase 1; they
  touch different code paths (checker dispatch vs. test harness).
- **Relative Effort**: M — threading new parameters through the splice path
  and adding dispatch logic. Roughly a week.
- **Difficulty**: hard — threading new state through shared control flow
  (the splice path visited by `check_terms_relaxed`), with bound resolution
  that must match the non-combinator case exactly. The brief identifies this
  as "the real work" of item 3.
- **Open Questions / Blockers**: None remaining. Probe 4 resolved the
  delivery question: the resolved `impl:` symbol for a bare member call is
  delivered via the per-splice `CallInst`'s `trait_calls` map from Phase 1
  (the same channel used for case (a) bounded poly words). Phase 3 seeds
  `trait_calls` via `poly_trait_member_call` + `resolve_user_bound` at the
  splice site; the per-splice record carries it to lowering.

### Phase 4: Reject bound dispatch inside materialized quotations (option b)

- **Goal**: Bound dispatch inside a materialized quotation within a bounded
  combinator is rejected with a located error rather than silently
  miscompiled.
- **Requirements Covered**: R5
- **Scope**:
  - `src/check/` (the materialization detection path) — add a rejection when
    a bounded combinator's body materializes a quotation that dispatches a
    bound member. The rejection is placed at the point where materialization
    is detected in a bounded-combinator context. Candidate locations:
    `src/check/audits.rs:463` (`reject_poly_quotation_anywhere`, one of the
    three gates), or the materialization boundary in
    `src/check/terms.rs` where `call` on a materialized runtime quotation is
    rejected (line ~1239, `call_needs_quotation_error`).
  - `src/ir/func_builder/mod.rs:1061` (`lower_materialized`) — context for
    understanding why the rejection is correct: the materialized quotation gets
    its own `IrFunc` with a fresh `FuncBuilder` and `inline_uid: 0`, so two
    splices of the enclosing combinator would collapse to the same key.
  - A new test exercising the rejection. The case is currently unconstructible
    (three independent gates block it: the audit gate at
    `src/check/audits.rs:463`, the `x__inl0` capture gate, and the `call`
    gate at `src/check/terms.rs:1239`). The test may need to bypass one gate
    to exercise the rejection, or test the rejection function directly with
    the inputs that would trigger it.
- **Explicitly out of scope for this phase**:
  - Any compiler changes beyond the rejection and its test.
  - The per-splice instantiation mechanism — phase 1.
  - The dispatch injection — phase 3.
- **Entry Conditions**: Phase 3 complete — the dispatch injection detects
  bare member calls in the splice path, which is what the rejection builds on
  (it rejects the specific case of bound dispatch inside a materialized
  quotation).
- **Exit Criteria / Verifiable Artifacts**:
  - Bound dispatch inside a materialized quotation from a bounded combinator
    is rejected with a located error naming the combinator and explaining that
    bound dispatch in materialized quotations is unsupported.
  - The motivating program (whose `~[ ]` arms are spliced, not materialized)
    is not affected — the oracle harness from phase 2 still passes.
  - `cargo fmt --check && cargo clippy -- -D warnings && cargo test` passes
    with no regressions.
- **Parallelism**: SEQUENTIAL after phase 3 — the rejection requires dispatch
  detection from phase 3.
- **Relative Effort**: S — a located rejection guard with a test. A day or
  two.
- **Difficulty**: standard — a located rejection guard. The case is currently
  unconstructible, so the rejection is a safety net, not a hot-path change.
- **Open Questions / Blockers**: The exact placement of the rejection (which
  of the three gate sites, or a new check) is unresolved. The implementer
  should place it where materialization is detected in a bounded-combinator
  context, and ensure it does not fire for spliced (non-materialized) `~[ ]`
  arms.

### Parallelism Summary

- Phase 1: SEQUENTIAL (must be first).
- Phase 2: PARALLEL with phase 3 (both depend only on phase 1).
- Phase 3: PARALLEL with phase 2 (both depend only on phase 1).
- Phase 4: SEQUENTIAL after phase 3.

### Effort Summary

- Phase 1: M (core fix, checker-lowering interface)
- Phase 2: S (test harness, flag flip)
- Phase 3: M (dispatch injection, threading)
- Phase 4: S (rejection guard, test)
- Total: 2M + 2S

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Per-splice instantiation records", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "Oracle harness and motivating program", "effort": "S", "difficulty": "standard" },
    { "phase": 3, "focus": "Dispatch injection into splice path", "effort": "M", "difficulty": "hard" },
    { "phase": 4, "focus": "Reject materialized quotation dispatch", "effort": "S", "difficulty": "standard" }
  ]
}
```
