# Spec: P7.S3o — Bound dispatch on a poly combinator's own type variable

**Status:** Implemented
**Created:** 2026-08-26
**Discovery:** docs/roadmap/P7/slice3o-brief.md

## Problem Statement

A combinator (an `inline` word) is spliced at its call sites — no call frame, no indirection. When such a combinator carries a `'T: TraitName` bound on its own type variable and its body calls a bare trait member (`cmp` directly, not the exported `gt` wrapper), the bound cannot dispatch: `reject_user_bound_on_combinator` rejects it at the gate before the body is ever checked. The fallback is to ship the combinator non-inline, paying a real call frame per instantiation — the shape S3s deliberately chose for `mymax`/`mymax3` to avoid this slice.

Behind that rejection lie three independent gaps: (1) a pre-existing span-keyed `insts` collision that silently miscompiles any poly combinator calling a poly word at two types — the real blocker, now guarded by the committed 1a overwrite-detector (507e0b7) which converts the miscompile into a located error; (2) the splice path (`check_terms_relaxed`) has zero bound-dispatch calls — `poly_trait_member_call` is never invoked there, so bare members read as "unknown word"; (3) nothing records an obligation for a combinator's body, so the resolution tables are empty. This slice closes all three gaps, rejects the materialized-quotation corner (option b, settled by probe 2), and validates the fix by differential-testing the inline motivating program (`mymax`/`mymax3`) against the non-inline baseline at two splices and two types.

## Requirements

- **R1.** A poly combinator (an `inline` word) whose body calls a polymorphic word, spliced at two or more distinct concrete types, must compile and run correctly — each splice's inner-call instantiation must be monomorphized independently, dispatching to the correct monomorph for each concrete type.
- **R2.** The per-splice instantiation mechanism must derive inner-call instantiations from the splice's θ (already computed by `check_poly_combinator_args`) on the check side, reusing `check_poly_call`'s existing unification + `instantiation_symbol` to mint a per-splice `CallInst` per inner poly call. This introduces no new check/lower key-consistency invariant beyond the existing `inline_uid` threading.
- **R3.** A bounded poly combinator (`'T: TraitName`) whose body calls a bare trait member (the member name directly, not a wrapper word) must resolve the member to the correct `impl:` at each concrete splice site, dispatching to the right implementation for each concrete type.
- **R4.** The bound-dispatch resolution at the splice site must be transitive over the splice tree: an unbounded combinator whose body splices a bounded one must not trigger a rejection for the inner combinator's bound member call, neither at the outer combinator's standalone check nor at the splice site.
- **R5.** Bound dispatch inside a materialized quotation within a bounded combinator must be rejected with a located error rather than silently miscompiled.
- **R6.** The motivating program `examples/poly_if.sth` with `mymax` and `mymax3` restored to `inline` with `'T: Copy Ord` must produce byte-identical stdout to the non-inline baseline when run at two concrete types (i64 and f64).
- **R7.** The oracle harness must validate the inline build against the non-inline baseline at two splices of the enclosing combinator (`mymax3`) and two concrete types, diffing both program stdout and resolved `impl:` dispatch targets via the `nm`/`objdump` call-graph walk already in the harness skeleton.
- **R8.** `tests/corpus_stdout/poly_if.txt` must remain byte-identical after `mymax`/`mymax3` are flipped to `inline`.
- **R9.** The fix must not change the compilation behavior of any existing program that does not splice a poly combinator calling a poly word at two types — the existing test corpus and golden tests must pass unchanged.
- **R10.** Every design shape in this spec must be validated with two splices of the enclosing combinator at two concrete types where one type matches the `i64` stand-in used by the standalone check (`check_poly_combinator_standalone`).

## Success Criteria

- [x] An unbounded inline combinator calling a poly word, spliced at i64 and f64, compiles and runs correctly — two distinct monomorphs emitted, each dispatching to the right callee (R1).
- [x] The `pid`/`c` fixture (currently hits the 1a guard) compiles and runs correctly instead of erroring (R1).
- [x] A bounded inline combinator calling a bare trait member (`cmp` directly) resolves to the correct `impl:` at i64 and f64 — two distinct monomorphs, correct output (R3).
- [x] An unbounded combinator splicing a bounded one does not reject the inner combinator's bound member call (R4).
- [x] Bound dispatch inside a materialized quotation from a bounded combinator produces a located error (R5).
- [x] `mymax`/`mymax3` inline with `'T: Copy Ord` produce byte-identical stdout to the non-inline baseline (R6, R8).
- [x] The oracle harness diffs the inline build against the non-inline baseline on both stdout and dispatch targets, at two splices and two types (R7).
- [x] `cargo fmt --check && cargo clippy -- -D warnings && cargo test` passes with no regressions (R9).
- [x] Every new golden test exercises two splices at two types, one matching `i64` (R10).

## Scope & Boundaries

**In scope:**

- Item 1c: per-splice instantiation records via splice log + check-side derivation (the hybrid of shape iii from the brief, refined by round-4 probe 3). The checker logs each splice's θ and derives the inner-call `CallInst`s from that θ on the check side — redirecting `check_poly_call`'s existing `CallInst` mint — not on the lowering side.
- Item 3: dispatch injection into `check_terms_relaxed` — threading `poly_trait_member_call` plus the `sig`/`TraitCtx` needed to resolve a bare trait member at the splice site. The transitive skip via a pre-pass body scan of the combinator's own `WordDef` (not a `Provenance` flag).
- Option (b): reject bound dispatch inside materialized quotations from bounded combinators with a located error and a test.
- Oracle harness: flip `mymax`/`mymax3` to inline, add a fixture calling both words at two types, diff against the non-inline baseline.
- Removal of `reject_user_bound_on_combinator`'s call site (the gate that currently rejects all `Bound::User` on a combinator's own type variable).

**Out of scope:**

- Re-specifying the 1a overwrite-detector guard (already committed as 507e0b7; stays as a safety net).
- Modifying `poly_trait_member_call`, `resolve_user_bound`, `CallInst`, or the non-combinator `trait_calls` mechanism (P7.S3e) — these are reused as-is.
- Ambiguity/disambiguation rules for two bounds sharing a member name (P7.S3p's rulings inherited as-is).
- A hand-written native backend or WASM lowering (CLAUDE.md load-bearing invariants).
- In-process JIT or comptime interpreter (out of scope per CLAUDE.md).

## Solution Approach

The spec adopts a **hybrid of shape (iii)** for item 1c, refined by the round-4 probes. The checker logs each splice's θ (already computed by `check_poly_combinator_args` and currently discarded) and derives the inner-call `CallInst`s **on the check side**, not the driver side. This avoids the `pub(super)` visibility barrier (`check_poly_call` and `unify_poly_input` are `pub(super)`, inaccessible from the driver) and reuses the existing `discover_transitive_instantiations` fixpoint for transitive discovery: the derived per-splice `CallInst`s feed the fixpoint naturally, preserving poly chains (combinator → p1 → p2). The probe confirmed the existing `poly_cross_calls`/`compose` mechanism cannot be used as-is, but the fixpoint's seeding logic is reusable once the `compose` combinator-callee rejection is relaxed/bypassed for splice-derived records — not merely by inserting into the span-keyed `module.instantiations`, which would re-introduce the 1a collision.

The approach adds no new check/lower key-consistency invariant: the `inline_uid` already threads both sides, and resolution happens where θ is actually known rather than where spans collide. The 1a overwrite-detector guard (507e0b7) stays as a safety net but is no longer triggered for spliced-combinator inner calls because the checker skips the span-keyed `insts.insert` for those calls. Removing `reject_user_bound_on_combinator`'s call site unblocks bounded combinators: the standalone check already substitutes `i64` and checks bounds against the real `TraitResolveCtx`, so a body calling a real poly word (`gt`) passes; a body calling a bare member (`cmp`) fails with a legible "unknown word" error until dispatch injection lands.

**Bounded inner words — two cases (probe 4 finding).** "Bounded inner word" splits into two cases with different check-time behavior:

- **Case (a): bounded poly word** (e.g. `gt`, a regular poly word with `'T: Ord` that internally calls `cmp`). `trait_calls` IS seeded at check time by `resolve_user_bound` inside `check_poly_call` at each splice site. The per-splice records carry the seeded `trait_calls` map, and lowering dispatches correctly. The motivating program (`poly_if.sth`) calls `gt`, not `cmp` directly, so Phase 1 + Phase 2 suffice for it.
- **Case (b): bare member call** (e.g. `cmp` called directly from a combinator body). `trait_calls` is NOT seeded — `check_terms_relaxed` lacks `poly_trait_member_call` (Gap 2) and no obligation is recorded (Gap 3). This case genuinely needs Phase 3's dispatch injection. It is the hot-path optimization (calling `cmp` directly instead of through `gt`).

For item 3, the spec threads `poly_trait_member_call` and the `sig` (`PolySig` carrying the bounds) plus the `TraitCtx`/`TraitResolveCtx` into `check_terms_relaxed` — the splice path that currently has zero bound-dispatch calls. At the splice site, θ is concrete (from `check_poly_combinator_args`), so a bare member call resolves to the correct `impl:` per splice, reusing `resolve_user_bound`'s `impl:`-registry lookup. The transitive skip is a **pre-pass body scan**, not a runtime `Provenance` flag: the standalone check runs in the pre-pass with no splice active, so a flag set during splicing would never be set when the standalone check runs. Instead, before the i64 body walk, the standalone check scans the combinator's own `WordDef` for calls to bare trait members (names matching a `Bound::User` member in the combinator's `poly` sig) and skips the i64 walk for those terms — they are checked at the splice site where θ is concrete. Nested combinators are handled naturally: each combinator's standalone check skips its own member calls, and the splice walk resolves them at the concrete splice θ.

For option (b), the spec rejects bound dispatch inside materialized quotations from bounded combinators with a located error. Probe 2 confirmed this is sound: the motivating program's `~[ ]` arms are spliced by `branch`/`lower_if` into basic blocks (never materialized — zero `__quot` symbols), and the case the rejection targets is currently unconstructible (three independent gates block it) but correct to reject when it becomes constructible, because the materialized quotation gets its own `IrFunc` with no splice-site prefix and two splices would collide.

The oracle harness (`tests/phase7_slice3s_oracle.rs`) already builds `examples/poly_if.sth` and diffs against itself, proving the plumbing works. S3o gives it a real second variant: a non-inline baseline vs. the inline candidate. A new fixture calling both `mymax` and `mymax3` at two types is needed because `main` currently calls `mymax3` only — `mymax` mints no monomorph and the harness never sees it.

## Open Questions

All resolved by round-4 probes:

- **How lowering derives inner-call instantiations from θ** — derivation stays on the check side (not the driver), avoiding the `pub(super)` visibility barrier. The checker uses the splice θ directly (already computed by `check_poly_combinator_args`) to mint per-splice `CallInst`s, and feeds them into the existing `discover_transitive_instantiations` fixpoint for transitive discovery.
- **How the resolved `impl:` symbol for a bare member call is delivered to lowering** — "bounded inner word" splits into two cases. Case (a) — bounded poly words like `gt`: `trait_calls` IS seeded at check time by `resolve_user_bound` inside `check_poly_call` at each splice site, so per-splice records carry it naturally. Case (b) — bare member calls like `cmp`: `trait_calls` is NOT seeded (Gap 2 + 3); Phase 3's dispatch injection seeds it, and the resolved `impl:` is delivered via the per-splice `CallInst`'s `trait_calls` map from Phase 1.
- **Whether option (b) blocks the motivating program** — the motivating program's `~[ ]` arms are spliced, not materialized (zero `__quot` symbols). Option (b) is sound and does not block.
- **Whether the `i64` stand-in check's `insts` are a writer** — the build path already isolates the stand-in's `insts` into a scratch `HashMap`. The collision is purely between real splices.
- **Whether the materialized-quotation corner is constructible** — three independent gates block it. The corner is gated-but-watched, not a blocker.

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| The check-side derivation of inner-call instantiations from θ is incorrect or incomplete | Med | The oracle harness (phase 2) and the two-splice/two-type testing discipline (R10) validate every shape. The `pid`/`c` fixture is the minimal two-type probe. |
| Removing `reject_user_bound_on_combinator` exposes new failure modes for bounded combinators calling bare members | Med | The standalone check catches bare member calls with a legible error until phase 3 fixes them. No existing program is affected (the rejection prevented them). |
| The transitive skip mechanism is incorrect | Low | The transitive skip is a pre-pass body scan (not a runtime flag), scoped to the combinator's own `WordDef`. Nested combinators are handled naturally. The two-splice discipline validates it at depth two. |
| The materialized-quotation rejection fires for the motivating program | Low | Probe 2 confirmed zero `__quot` symbols for the motivating program. The rejection targets only materialized quotations. |
| The per-splice instantiation mechanism changes the compilation of existing programs | Low | R9 (no regression) and the existing test corpus. The mechanism only affects spliced-combinator inner calls; existing combinators call builtins/intrinsics/quotation parameters, not plain poly words. |
| Bounded combinators calling bare members ICE between Phase 1 and Phase 3 | Med | Case (a) bounded poly words like `gt` are handled by Phase 1. Case (b) bare member calls like `cmp` need Phase 3; the intermediate-state test pins case (b) as a legible error, not an ICE. |

## Implementation

| Area | Commit | Key files |
|------|--------|-----------|
| **Per-splice instantiation records** (item 1c, hybrid shape iii): per-splice `CallInst` derivation on the check side, removal of `reject_user_bound_on_combinator`'s call site, `splice_records` on `Module`, `inline_uid`-keyed lowering lookup, transitive discovery preservation | `bc02b43c` | `src/ast.rs`, `src/check.rs`, `src/check/combinators.rs`, `src/check/declarations.rs`, `src/check/engine.rs`, `src/check/poly.rs`, `src/driver.rs`, `src/ir.rs` |
| **Oracle harness + motivating program flip**: `mymax`/`mymax3` restored to `inline` with `'T: Copy Ord`, harness diffs inline vs. non-inline baseline on stdout and dispatch targets at two splices and two types | `c4bd3af6` | `examples/poly_if.sth`, `src/check/combinators.rs`, `tests/phase7_slice3s_oracle.rs`, `tests/qbe_baseline/poly_if.ssa` |
| **Dispatch injection into splice path** (item 3): `poly_trait_member_call` + `TraitCtx`/`TraitResolveCtx` threaded into `check_terms_relaxed`, pre-pass body scan for transitive skip, resolved `impl:` delivered via per-splice `CallInst`'s `trait_calls`, nested combinator support | `05b3c919` | `lib/cmp.sth`, `src/ast.rs`, `src/check.rs`, `src/check/combinators.rs`, `src/check/declarations.rs`, `src/check/poly.rs`, `src/check/terms.rs`, `src/driver.rs` |
| **Reject bound dispatch inside materialized quotations** (option b): located rejection guard for bounded combinators that materialize quotations dispatching bound members | `0edad1a5` | `src/check.rs`, `src/check/captures.rs`, `src/check/combinators.rs`, `src/check/engine.rs`, `src/check/poly.rs`, `src/check/terms.rs`, `tests/phase4_combinators.rs` |
