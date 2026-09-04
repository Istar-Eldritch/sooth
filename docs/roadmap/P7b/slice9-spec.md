# P7b.S9 spec — module-aware trait-impl matching

Scoped against worktree `p7b-s9`, HEAD `600bc1b`, baseline `cargo test
--no-fail-fast` **82 binaries, 3149 passing + 1 known-flaky, 0 unconditionally
failed** (see R-NFR5 for the flaky test and why). Discovery input:
[slice9-brief](./slice9-brief.md) (recon round) and its two completed evidence
rounds — the adjudicated mechanism in [slice9-probes](./slice9-probes.md)
(verbatim probe log + verdict) and the validated golden designs in
[slice9-paper-tests](./slice9-paper-tests.md). S9 is the
[post-ship correction](./slice5-spec.md) that closed S5: the tier policy S5
built governs `env`/`select_overload` ctor-*construction* selection only, and
the cross-pick it was motivated by lives in two other mechanisms, both upstream
or downstream of the (sound) trait-impl matcher.

> **Mechanism correction (carried from the probe round; supersedes the
> roadmap).** The roadmap's S9 entry claims "no module-identity check anywhere
> in `match_impl_target` or the `select_most_specific` tie-break". **That is
> false.** The probe round proved (V1/F1/F3) that `match_impl_target_rec`'s
> `Generic` arm *does* compare header identity `(idx, module)` and resolves
> per-module correctly in every run; the matcher and pattern resolution are
> sound. The real defects are two, both off the matcher:
>
> - **V2 (operand provenance, deterministic `pb2` `2\n2`).** A bare,
>   un-annotated ctor call silently borrows an *unrelated* module's
>   eagerly-minted instantiation. In `pb2` only `b`'s `usesize` spells
>   `Widget[i64]`, so exactly one `Widget[i64]` mint exists program-wide
>   (`StructId(2)`, provenance `gi=1, module=4` — b's header). a::run's bare
>   `Widget` ctor call applies *that* mint, so both callers dispatch with
>   provenance `(1, 4)` and only b's pattern matches. The S5 tier policy never
>   fires — `ctor-select` shows a single candidate at a's call site (a's own
>   ctor is absent from the candidate set there).
> - **V3 (monomorphization identity, non-deterministic `mk` variant
>   `1\n1`/`2\n2`).** `trait_calls: HashMap<Span, String>` is **not** shared
>   across groundings — it is a local created fresh per instantiation
>   (`src/check/poly.rs:7235`) and moved onto that instantiation's own
>   `CallInst` (`:7384`; field `CallInst.trait_calls`, `src/ast.rs:2808`), so
>   two groundings write two separate maps and never overwrite each other.
>   The actual collapse is one stage later, in lowering's instantiation dedup
>   (`src/ir/driver.rs:350-373`): a `HashSet<String>` keyed on
>   `instantiation_symbol(&inst.callee, &inst.subst)`, iterating
>   `module.instantiations.values()` (a randomized `HashMap`). `sized`'s two
>   groundings have different substitutions (`Struct(StructId(2))` vs
>   `Struct(StructId(3))`) but `instantiation_symbol`'s rendered-name fall-through
>   arm (`other => other.name().to_string()`, `src/ast.rs:2886`) is
>   non-injective on them, so both mint the identical symbol
>   (`sooth_mono_sized__m2__t0_Widget_i64_`); the `HashSet::insert` in the dedup
>   loop keeps only the first `CallInst` reached under iteration order —
>   **discarding the other grounding's entire `CallInst`, `trait_calls` map
>   included**, not overwriting one entry within a shared map. `instantiation_symbol`
>   (`src/ast.rs:2869`) *is* the mono key already (its own doc: "the checker's
>   call-site table and the lowered `IrFunc.name` are minted from one source of
>   truth", `:2864-2866`) — there is no separate key to bring up to its
>   discipline; the fix widens this one function's **fall-through** arm (the
>   `CtorImage` arm is already correct and stays as-is) to cover the
>   `Struct`/`Enum` case the same way. The race is in **lowering's dedup loop**
>   (`src/ir/driver.rs:359`), which iterates the checker-built
>   `module.instantiations` map (a compile-time `HashMap`, built deterministically
>   but iterated in randomized order, once per `sooth build` process): one built binary
>   reruns deterministically (measured: 5/5 stable repeat runs on each of two
>   builds); only rebuilds flip (measured: 6/8 `1\n1`, 2/8 `2\n2` across 8
>   rebuilds). (`H1`/`H5` are the same mechanism seen from the specialization
>   side and the dedup side — one finding, not two. The earlier reading of this
>   mechanism as `trait_calls.insert(ob.span, symbol)` being "last-writer-wins
>   across two groundings" was wrong — that call writes into a per-instantiation
>   map, never a shared one; see the errata in
>   [slice9-probes](./slice9-probes.md).)
>
> This spec **rules** on all five decisions the brief hands the spec-writer.
> The roadmap's own sentences are corrected at slice exit (R7).

---

## Rulings

### R1 — V2 fix site: pin the missing candidate first, then ground bare ctors at the caller's own header. (Phase 1 → Phase 2)

**R1.0 (Phase 1, blocking gate).** Before choosing the V2 fix site, pin *why*
a's own header's ctor is absent from the single-candidate set at a's bare
`Widget` call in `pb2`. The probe recorded the symptom (`ctor-select` fires with
one candidate, provenance `(gi=1, module=4)` = b's) but not the registration
cause. The candidate set for a bare ctor application is fed from
`struct_generated_sigs` registration (`src/check/declarations.rs:1824`) into
either the single-candidate arm (`src/check/terms.rs:932`) or the tier-selection
path (`src/check/terms.rs:968-991`, `select_overload_fallback_sourced`/
`select_overload`). The Phase-1 deliverable is a
recorded verdict answering: does a's own `Widget[i64]` instantiation *exist* in
the registry at that call site (and the candidate scan filters it out), or is it
*never minted* because a's construction is bare/inferred and only b's `usesize`
forces an eager mint? The two answers point at different fix sites (R1.1a vs
R1.1b); Phase 1 chooses between them with evidence, not prose.

**R1.1 (Phase 2).** A bare ctor application must ground at the **caller's own
resolved header** — the same module-scoped resolution the parser already applies
to type positions (F3, `poly_generic_header` `src/parser.rs:7101` via
`bare_generic_owner` `:7133`) — minting its own instantiation when none exists
for that header, and **never** silently substituting another module's
eagerly-minted instantiation. Two candidate sites, chosen by R1.0's verdict:

- **R1.1a (registration/application, if a's mint is absent):** the bare-ctor
  candidate registration/application path must mint (or select) the instantiation
  keyed to the caller's own header, so a's call sees a's own `Widget[i64]`
  candidate. Neighbourhood: `struct_generated_sigs`
  (`src/check/declarations.rs:1824`) and the single-candidate arm
  (`src/check/terms.rs:932`).
- **R1.1b (operand normalization, if a's mint exists but is filtered):** the
  operand handed to `find_bound_impl` must carry the caller's own header
  provenance rather than the borrowed mint's. Neighbourhood: the operand
  normalization feeding `find_bound_impl` (`src/check/poly.rs:8235`).

**The matcher is not touched.** `match_impl_target`/`..._rec` (`:8833`/`:8846`)
and `find_bound_impl`'s registry scan (`:8235`) are sound (F1/F2/V1) and stay
as-is: with identity-correct operands, distinct headers have distinct
`(idx, module)` and exactly the right pattern matches.

### R2 — V3 fix shape: widen `instantiation_symbol` to be injective on grounding identity. (Phase 3)

V3 is **one mechanism, one fix site** — not two coupled halves. `trait_calls`
is per-instantiation (`CallInst.trait_calls`, `src/ast.rs:2808`; created fresh
at `src/check/poly.rs:7235`, moved onto the instantiation at `:7384`) and is
already lowering-consumed (`src/ir/driver.rs:414`,
`src/ir/func_builder/calls.rs:375/385`, `src/ir/destructors.rs:379` via
`empty_trait_calls` `src/ir.rs:130`); it is never re-keyed by this fix (see the
withdrawal note below — doing so would breach R-NFR1).

**R2.1 — the fix.** `instantiation_symbol` (`src/ast.rs:2869`) IS the mono key
(its own doc comment: "the checker's call-site table and the lowered
`IrFunc.name` are minted from one source of truth", `:2864-2866`) — there is no
separate key elsewhere to widen. Its `Type::CtorImage` arm already keys on
`GenericId` (`:2521`); its fall-through arm for every other `Type`
(`other => other.name().to_string()`, `:2886`) renders only the type's name,
which collapses `sized`'s two `Widget[i64]` groundings
(`Struct(StructId(2))`/module 3, `Struct(StructId(3))`/module 4) to the
identical string `"Widget_i64_"`. The fix widens that one fall-through arm so a
`Type::Struct`/`Type::Enum` operand renders its own carried id
(`StructId`/`EnumId`'s inner index — `Type::Struct(StructId, &'static str)`,
`src/ast.rs:2991`, already holds it in the matched variant) into the symbol,
the same way the `CtorImage` arm already renders its carried `GenericId`. No
lookup, no new parameter, no signature change: `instantiation_symbol(word:
&str, subst: &Subst)` keeps its signature, so all five call sites
(`src/ir/driver.rs:109/316/365/851`, `src/ir/func_builder/calls.rs:280`) keep
compiling unchanged — `GenericTypes::struct_instantiation_of`/`enum_instantiation_of`
are a different subsystem (the matcher's own provenance lookup, F1) and are not
needed here, since `Module::structs`/`Module::enums` are whole-program-assembled
registries and a `StructId`/`EnumId` is already globally unique across modules
on its own — no module field is needed either. This makes two groundings mint
two distinct symbols, so lowering's dedup (`src/ir/driver.rs:350-373`) keeps both `CallInst`s
— including both `trait_calls` maps — and each compiles its own `sized` body
with its own single `size` call resolved to its own grounding. No IR/lowering
edit: the dedup loop and every `trait_calls` consumer are untouched: they
simply stop colliding.

**R2.2 — withdrawn.** An earlier reading of this defect proposed re-keying
`trait_calls`/`builtin_overloads` from `Span` to a grounding-aware key. That is
both inert and out of bounds: `trait_calls` never collides across groundings in
the first place (each instantiation owns its own map, per `R2` above), so
re-keying it changes nothing while the two `CallInst`s still collide on one
`instantiation_symbol`; and `trait_calls` is lowering-consumed (`CallInst`,
above), so editing its key shape would touch `src/ir/driver.rs` and
`src/ir/func_builder/calls.rs`, which R-NFR1 forbids without an explicit stop-
and-escalate. There is no map-side fallback for R2.1; if widening
`instantiation_symbol` reds unrelated goldens, stop and escalate per R-NFR1
rather than reach for R2.2.

**R2.3 — verification, not a coupling choice.** Phase 3 verifies G2 resolves
to deterministic `1\n2` under the R2.1 widening alone; there is no second
mechanism to justify or fall back to. A committed test must **never assert a
run-count ratio** (the pre-fix flip is nondeterministic; see R-NFR3).

### R3 — D3: third-module mono caller — Phase 4 determines the outcome, does not assume it. (Phase 4)

**This is a determination, not a confirmation.** R1.1's own rule is "ground at
the caller's own resolved header" — but `c` declares no `Widget` header at all,
and the probe round already measured that a third module naming `Widget[i64]`
explicitly (with no declaring/importing header of its own) is a hard
`error: unknown type \`Widget\`` (probes P5a-ii). So R1.1's fix may not give `c`
a 2-candidate collision to begin with; Phase 4 must build the fixture against
the Phase-2/3 fix and observe which of two outcomes actually happens, then pin
that:

- **(a) 2-candidate collision.** If `c`'s bare `Widget` ctor call *does* surface
  a 2-candidate collision (e.g. because both `a` and `b`'s mints are visible
  through `c`'s wildcard imports and the fix's own-header grounding degrades to
  "ambiguous among visible headers" rather than "no header"), it lands in S5's
  `select_overload` tier policy; `c` is neither declaring module, so it falls to
  the ambiguity tier → **compile-time error**. Post-fix assertion: `build_error`
  pinning a located message naming `Widget`, both candidate modules, and `c`'s
  call site — pinned against the fixture's actual output, not re-derived here.
- **(b) no header, earlier error.** If `c`'s bare ctor grounds against no
  visible header at all (consistent with P5a-ii and R1.1's own-header rule),
  it errors earlier than dispatch, with whatever message the mechanism actually
  produces (e.g. `unknown type` or a checker error for an unresolvable bare
  ctor). Post-fix assertion: `build_error` pinning that measured text.

Either outcome satisfies the roadmap's "real ambiguity error, not a silent
guess" intent (a caller that cannot resolve one header among several visible,
equally-plausible ones does not get a silent pick). Phase 4's first task is to
build the fixture and observe; only then choose which of (a)/(b) to pin.

### R4 — D4: no new dispatch-time visibility/tier machinery in the registry scan. (design ruling; no code)

Ruling: **no new visibility filter or dispatch-time tier machinery in
`find_bound_impl`.** With identity-correct operands (R1), distinct modules'
headers never both match one operand — their `(idx, module)` identities differ —
and the import system prevents a caller from ever holding an operand whose header
identity is ambiguous to it (probes P5b/P5c: import cycle / placement rule /
selective-import collision all fire first). The one **constructible**
concrete-target ambiguity — two modules each declaring a blanket `impl: Sized
for 'T` — already errors at declaration time via `check_impl_decls`
(`src/check/declarations.rs:544`, P7.S4 R7), module-blind by design because a
bare `PolyType::Var` carries no header identity. `ImplDecl.module`
(`src/ast.rs:2492`) stays read only by the placement rule (`:588`); the scan
stays `trait_id` + pattern-match only. Under R3 outcome (a), the third-module
ambiguity routes through the ctor `select_overload` path, not `find_bound_impl`;
under outcome (b), `c` errors before reaching either registry. Either way,
`find_bound_impl` gains no new machinery — revisit only if Phase 4's evidence
somehow forces candidate-set awareness at dispatch, which neither outcome does.

### R5 — Regression pins stay green, unchanged. (every phase)

- **Golden #10** `same_named_ctors_in_two_modules_dispatch_distinct_impls`
  (`tests/phase7b_slice2.rs:655`) — the only existing golden exercising
  per-operand trait-impl dispatch with **distinct** substitutions (`i64`/`str`)
  across modules. Must keep printing `1\n2`. The fix repairs the
  *same*-substitution (`pb2`) case without regressing the distinct-substitution
  case.
- **S5 tier-1** `cross_module_same_shaped_ctor_dispatches_callers_own_impl`
  (`tests/phase7b_slice5.rs:100`) — pins S5's ctor/destructure tier policy
  (`select_overload`), a different registry from S9's. Must keep printing
  `15\n25`.
- **`same_named_ctor_mk_ambiguity_resolves_but_impl_dispatch_still_cross_picks`**
  (`tests/phase7b_slice4.rs:427-490`) — **not currently green**: same shape as
  G2 (`mk` variant, two modules, impl constants 1/2), different trait
  (`Functor`, not `Sized`) and text — the slice4 test is re-pinned in Phase 3,
  G2 is written fresh in `tests/phase7b_slice9.rs`; do not churn one fixture to
  match the other. Currently hard-pinned to the pre-fix
coin-flip `1\n1` (measured: 3/8 red on rerun — the committed ratio assertion
R-NFR3 forbids). Phase 3 re-pins it to deterministic `1\n2`, renames it off
its dead pre-fix criterion, and rewrites its comments (`:423-425`, `:485-488`),
which narrate the falsified matcher-blindness story, to the corrected
attribution (see Phase 3's authorization below); until
  Phase 3 lands, every other phase's green gate may see this test red
  independently of that phase's own changes — note it, do not chase it.
- **G3** `duplicate_blanket_impl_across_modules_is_a_declared_error` — the
  declaration-time duplicate check (`check_impl_decls`, `:544`) must survive the
  fix untouched. New regression golden in `tests/phase7b_slice9.rs` (not new
  work; it pins pre-existing behaviour).

### R6 — Growth-structure re-check at phase exit. (final phase)

`src/check/poly.rs` is ~21k lines; its split remains deferred (3/5 signals, no
clean cut — recorded S5 residual). S9 is expected to touch `poly.rs` (obligation
wiring / mono identity), the ctor path in `terms.rs`/`declarations.rs`, possibly
`ast.rs` (identity helpers), and `tests/phase7b_slice9.rs` (new). Re-run the
CLAUDE.md split signals at phase exit against the files as they then stand; do
**not** preemptively split.

### R7 — Roadmap correction at slice exit. (final phase)

Five edit targets, all falsified by the probe round, none currently instructed
together — a prior draft of this ruling named only target 1:

1. **`docs/roadmap/P7b-higher-kinded-types.md:221-238`, mechanism sentence one**
   ("no module-identity check anywhere in `match_impl_target` or the
   `select_most_specific` tie-break…"): replace with the adjudicated two-defect
   story (operand provenance; monomorphization identity, one fix site —
   `instantiation_symbol`). State explicitly that the matcher and pattern
   resolution are sound.
2. **Same roadmap entry, `:232-234`, mechanism sentence two** ("it is
   `find_bound_impl`'s target-pattern matching itself being blind to which
   module's struct declaration a `impl: Trait for X` pattern was written
   against"): this is a *separate* sentence from target 1 and is equally
   falsified (F1/V1: the matcher's `Generic` arm already compares
   `(idx, module)`); replace or remove it in the same edit.
3. **Stale anchor `poly.rs:8218`** (appears in both the roadmap entry and
   `slice5-spec.md:776`, target 4 below): the real `fn find_bound_impl` is
   `src/check/poly.rs:8235`. Fix at both sites while editing them.
4. **`docs/roadmap/P7b/slice5-spec.md:776-778`** ("`pb2`'s actual collision is
   in `find_bound_impl`'s trait-impl target matching (`poly.rs:8218`), a
   separate, module-blind registry this phase never touches"): falsified by
   V1/V2 (the matcher is sound; the defect is operand provenance, upstream of
   the matcher). Correct in place, keeping the post-ship-correction marker
   structure (this is historical review-round prose, not the roadmap's own
   current-design section — mark the correction rather than silently rewriting
   the historical record).
5. **`docs/roadmap/P7b/slice5-spec.md:783`** ("also regressed … to a silent
   `1 1` at exit 0"): this is where the "silent `1 1`" claim actually lives
   (not in the roadmap entry, which never mentions `mk` or `1 1`) — correct to
   **nondeterministic** `1\n1`/`2\n2` (checker-HashMap-iteration-order
   dependent; a built binary is stable, rebuilds flip), same marker discipline
   as target 4.

For targets 1-2 (the roadmap's own current-design prose), also reword the exit
criterion: the constructible ambiguity shape is covered by the
**declaration-time** duplicate check (no new dispatch-time mechanism); the
third-module mono caller is ruled per R3's Phase-4 determination (either
outcome). Do not imply "add a dispatch-time ambiguity error" unless R3's Phase-4
evidence actually lands on outcome (a).

Per [[feedback_roadmap_design_no_history]]: the roadmap's own current-design
prose (targets 1-2) states the current design only, no "was X, now Y"
narration. Targets 4-5 are inside an already-historical "post-ship correction"
blockquote in `slice5-spec.md`; mark the correction there rather than deleting
the record of what the review round originally (incorrectly) believed.

---

## Requirements (traceable)

- **REQ-1** (R1.0): a recorded Phase-1 verdict on why a's own ctor is absent from
  the single-candidate set at a's bare `Widget` call in `pb2` (absent-mint vs
  filtered-mint), chosen with instrumentation evidence.
- **REQ-2** (R1.1): a bare ctor application grounds at the caller's own resolved
  header; it never substitutes another module's eagerly-minted instantiation.
- **REQ-3** (R2.1, observable): repeated `sooth build`s of unchanged source
  (the G2 shape — a shared bound word grounded at two same-rendered-name,
  distinct-identity `Struct`/`Enum` operands) produce identical dispatch
  decisions and output across builds; no HashMap-iteration-order dependence
  remains observable at the CLI.
- **REQ-4** (R2.1, mechanism): `instantiation_symbol` is injective on grounding
  identity — same rendered name, different `(StructId/EnumId)` ⇒ distinct
  symbols — so distinct groundings of a shared bound word compile to distinct
  specializations and lowering's dedup (`src/ir/driver.rs:350-373`) never
  collapses two of them into one. REQ-4 is the mechanism that delivers REQ-3.
- **REQ-5** (R2.3): both G1 and G2 resolve to deterministic `1\n2` under the
  R2.1 fix; no committed test asserts a run-count ratio; the pre-existing
  flaky pin (`tests/phase7b_slice4.rs:490`) is re-pinned in the same phase.
- **REQ-6** (R3): Phase 4 determines (does not assume) whether G4's after-column
  is a located compile-time ambiguity error naming `Widget`, both candidate
  modules, and `c`'s call site, or an earlier error from `c` grounding against no
  visible header; either is pinned against measured fixture output.
- **REQ-7** (R4): `find_bound_impl` gains no visibility filter or dispatch-time
  tier machinery; the declaration-time duplicate check is untouched.
- **REQ-8** (R5): #10 stays `1\n2`; S5 tier-1 stays `15\n25`; G3 pins the
  existing duplicate-impl error text.
- **REQ-9** (R7): all five roadmap/slice5-spec edit targets are corrected to
  the adjudicated story (not only the roadmap's first mechanism sentence).
- **REQ-10** (CLAUDE.md): every stage function edited gets a unit test beside it
  (happy path + one error/edge); every phase exit criterion is a golden.

## Non-functional requirements

- **R-NFR1 — check-stage only.** No IR change, no backend change, no lowering
  change. The linear spine, `Ptr[T]` opacity, QBE-only backend invariants are
  untouched (nothing here reaches lowering). If any candidate fix appears to need
  an IR/lowering edit, stop and escalate — that is out of scope.
- **R-NFR2 — matcher untouched.** `match_impl_target`/`..._rec` and
  `find_bound_impl`'s scan are expected to have **zero** behavioural diff. A
  Phase-exit check confirms no edit landed there (or, if one did, justifies it
  against F1/F2/V1).
- **R-NFR3 — never assert nondeterminism.** G2's pre-fix flip is HashMap-seed
  nondeterminism. The pre-fix evidence is noted in a scratch check (run N× before
  the fix), never encoded as a ratio assertion in committed tests. The committed
  G2 asserts only the post-fix deterministic `1\n2`.
- **R-NFR4 — green gate.** `cargo fmt --check && cargo clippy -- -D warnings &&
  cargo test` green at every phase exit; no dead parameters (clippy `-D
  warnings`).
- **R-NFR5 — baseline anchor.** Every phase is independently verifiable against
  the paper goldens with the baseline stated: **82 binaries, 3149 passing + 1
  known-flaky at HEAD `600bc1b`** (`tests/phase7b_slice4.rs:427-490`,
  `same_named_ctor_mk_ambiguity_resolves_but_impl_dispatch_still_cross_picks`,
  measured 3/8 red on rerun — same shape as G2, a different test file (R5); a
  single run may report `3150/0` by luck). Phases 1-2 may see this test red
  independently of their own changes; do not chase it. Phase 3 re-pins it (R5,
  REQ-5) to deterministic `1\n2`, after which the known-flaky pin is
  deterministically green and each subsequent phase's baseline is the prior
  phase's exit count (record the running total as each phase's goldens/units
  land; do not assert a fixed absolute).

## Success criteria (anchored in the validated goldens; before/after are measured facts, not re-derived)

| Golden | Name | Before (measured) | After |
| --- | --- | --- | --- |
| **G1** | `cross_module_same_shaped_impls_dispatch_each_callers_own_impl` (verbatim `pb2`) | `2\n2`, exit 0, deterministic (3 cycles) | `1\n2` |
| **G2** | `..._via_named_instantiation_dispatch_each_callers_own_impl` (`mk` variant) | **nondeterministic** `1\n1`/`2\n2` (8/2 paper, 6/4 probe — never asserted) | deterministic `1\n2` |
| **G2r** | `..._eager_minter_wins_regardless_of_caller` (a eager / b bare — cleanest V2 witness; **primary provenance regression pin**) | `1\n1`, exit 0, deterministic (5 cycles) | `1\n2` |
| **G3** | `duplicate_blanket_impl_across_modules_is_a_declared_error` | `error: duplicate \`impl:\` for \`'T\` (line 3, col 1); first declared at line 3, col 1`, exit 1 | unchanged (regression pin) |
| **G4** | `third_module_mono_caller_is_not_silently_cross_picked` | silent `2`, exit 0 (3 cycles) | `build_error` per R3's Phase-4 determination (outcome (a) or (b)) |
| **G5** | #10 (`tests/phase7b_slice2.rs:655`); S5 tier-1 (`tests/phase7b_slice5.rs:100`) | `1\n2`; `15\n25` | unchanged |

Fixture text for G1/G2/G2r/G3/G4 is preserved complete (every source file, not
prose-derived) in [slice9-paper-tests](./slice9-paper-tests.md), each built and
its before-column measured directly against this HEAD; use it as-is.

**Unit-level success criteria** (`thing_condition_expected` naming per
CLAUDE.md; fix sites are R1/R2's choice within the named function):

- `instantiation_symbol_same_rendered_name_different_struct_ids_mints_distinct_symbols`
  — two groundings with the same rendered type name but different
  `StructId`s (already globally unique across modules — no module component
  needed) must not collapse to one specialization (near `src/ast.rs:2869`);
- `bare_ctor_operand_provenance_is_callers_own_header_not_a_borrowed_mint` (name
  finalized once Phase 1 picks R1.1a vs R1.1b) — a lazily-minted, un-annotated
  bare-ctor operand's provenance is never borrowed from an unrelated eager mint;
- `check_impl_decls_duplicate_blanket_impl_across_modules_still_errors` —
  `check_impl_decls`'s blanket-impl duplicate check is unaffected
  (`src/check/declarations.rs:544`).

## Scope and boundaries

**In scope:** check-stage repair of V2 (operand provenance) and V3
(`instantiation_symbol`'s monomorphization-identity fall-through arm; obligation
wiring / `trait_calls` is untouched, see R2.2); Phase 4's D3 determination
(routes through S5's existing `select_overload` tier policy under outcome (a);
an earlier error under outcome (b) — R3); new `tests/phase7b_slice9.rs`; the
roadmap correction.

**Out of scope / untouched:**

- IR, lowering, backend — R-NFR1.
- `match_impl_target`/`match_impl_target_rec` and `find_bound_impl`'s registry
  scan — R-NFR2 / R4 (sound per F1/F2/V1).
- New dispatch-time visibility or tier machinery in the registry — R4.
- The declaration-time duplicate-impl check — R5/G3 (kept as-is).
- S5's `select_overload` tier *policy* — reused by R3 under outcome (a) only,
  not re-authored either way (R3 §).
- Any new trait surface, declaration syntax, or user-facing spelling.

## Codebase map (verified anchors, HEAD `600bc1b`, from the brief's machinery map)

- **V2 fix neighbourhood (ctor application).** single-candidate arm
  `src/check/terms.rs:932`; tier-selection path `src/check/terms.rs:968-991`
  (`select_overload_fallback_sourced`/`select_overload`); minted-candidate
  registration `struct_generated_sigs` `src/check/declarations.rs:1824`;
  module-scoped ctor name resolution `poly_generic_header` `src/parser.rs:7101`,
  `bare_generic_owner` `:7133`, unit-pinned
  `impl_target_module_generic_ctor_target_names_the_ctor_module`
  `src/check/declarations.rs:4355`.
- **V3 fix neighbourhood (one fix site).** `instantiation_symbol`
  `src/ast.rs:2869` — already the mono key (doc `:2864-2866`); `CtorImage` arm
  keys on `GenericId` already (`:2521`); the fall-through arm to widen is
  `other => other.name().to_string()` (`:2886`), to render the id already
  carried by `Type::Struct(StructId, ..)`/`Type::Enum(EnumId, ..)` (`:2991-2992`)
  — no lookup, no signature change (`GenericTypes::struct_instantiation_of`
  `src/ast.rs:1118`/`enum_instantiation_of` `:1128` are the matcher's own
  provenance lookup, F1, and are not part of this fix); `mangle`
  `src/resolve.rs:36`. Untouched-by-design: `CallInst.trait_calls`
  `src/ast.rs:2808` (created `src/check/poly.rs:7235`, moved onto the
  instantiation `:7384`); the dedup loop that stops colliding once R2.1 lands,
  `src/ir/driver.rs:350-373`; lowering consumers `src/ir/driver.rs:414`,
  `src/ir/func_builder/calls.rs:375/385`, `src/ir/destructors.rs:379`.
- **Matcher (untouched).** `find_bound_impl` `src/check/poly.rs:8235` (candidate
  loop `:8262-8276`, `candidate_bounds_discharge` `:8192`); `select_most_specific`
  `:8073` (`ambiguity_error` `:8099`, `specificity` `:9640`);
  `match_impl_target` `:8833` / `..._rec` `:8846` (`Generic` arm identity
  compare `:8995`). Call sites: inline-splice `:1722`, mono-member viability
  `:2239` (in `resolve_mono_member_call` `:2165`), where-clause discharge
  `:8192`, bound dispatch `:8408`.
- **D3 (tier policy, outcome (a) only).** S5's `select_overload` (in
  `src/check/builtins.rs` per S5 spec) and the ctor-select path feeding it from
  `terms.rs` — reached only if Phase 4 lands on outcome (a); under outcome (b),
  `c` errors before either registry is consulted (R3).
- **Impl record + duplicate check (untouched).** `ImplDecl` `src/ast.rs:2492`;
  placement rule `src/check/declarations.rs:575-588`; blanket-impl duplicate
  check `check_impl_decls` `:544`.
- **Whole-program env.** `poly_env` built once `src/check.rs:684-698`;
  `overload_symbols` `$$N` suffixing `src/check.rs:697`.
- **Goldens in play.** #10 `tests/phase7b_slice2.rs:655`; S5 tier-1
  `tests/phase7b_slice5.rs:100`; the pre-existing flaky pin
  `tests/phase7b_slice4.rs:427-490` (R5, re-pinned Phase 3); `pb2` fixture
  complete in [slice9-paper-tests](./slice9-paper-tests.md) G1.
- **Roadmap entries to correct (R7, five targets).**
  `docs/roadmap/P7b-higher-kinded-types.md:221-238` (two falsified mechanism
  sentences, one stale anchor); `docs/roadmap/P7b/slice5-spec.md:776-778` and
  `:783` (falsified post-ship-correction claims, same stale anchor).

## Open questions and risks

- **OQ-1 (resolved by Phase 1, gates Phase 2).** Absent-mint vs filtered-mint at
  a's bare ctor call (REQ-1). The V2 fix site (R1.1a vs R1.1b) depends on the
  answer; Phase 2 does not start the fix until Phase 1 records the verdict.
- **OQ-2 — resolved (no longer open).** V3 is one mechanism, one fix site
  (`instantiation_symbol`'s fall-through arm, R2.1); `trait_calls` never
  collides across groundings, so there is no second, map-side change to
  justify or fall back to (R2.2 withdrawn).
- **OQ-3 (resolved by Phase 4).** Does the V2 fix surface `c`'s bare ctor as a
  2-candidate `select_overload` collision (R3 outcome (a)), or does `c` ground
  against no visible header and error earlier (R3 outcome (b), consistent with
  probes P5a-ii)? Phase 4 builds the fixture first and pins whichever the
  mechanism produces — do not assume (a).
- **RISK-1.** Widening `instantiation_symbol`'s fall-through arm (R2.1) has
  blast radius across every monomorphized bound word, every `Struct`/`Enum`
  grounding, not just `sized`. Mitigation: R-NFR5 baseline anchor after each
  phase; if the widening reds unrelated goldens, **stop and escalate per
  R-NFR1** — there is no map-side fallback (R2.2 is withdrawn, not a narrower
  alternative).
- **RISK-2.** The V2 fix could perturb #10's distinct-substitution dispatch
  (F4). Mitigation: R5/G5 keeps #10 green as a per-phase gate.
- **RISK-3 (`poly.rs` size).** Editing the ~21k-line `poly.rs` again; the split
  stays deferred (R6). Do not preemptively split; re-run signals at exit.

---

## Phased delivery plan

Baseline for every phase: **82 binaries, 3149 passing + 1 known-flaky at HEAD
`600bc1b`** (R-NFR5). Each phase is independently verifiable against the paper
goldens and leaves the tree green (`cargo fmt --check && cargo clippy -- -D
warnings && cargo test`); Phases 1-2 may see the known-flaky test red
independently of their own changes.

- **Phase 1 — pin the missing candidate (V2 diagnosis gate).** Instrument the
  bare-ctor candidate path (`struct_generated_sigs` `declarations.rs:1824`;
  single-candidate arm `terms.rs:932`; tier path `terms.rs:968-991`) in `pb2`
  and record the REQ-1 verdict (absent-mint vs filtered-mint) in a new
  "## Phase-1 verdict (R1.0)" section appended to `slice9-probes.md`, committed
  together with Phase 2. Instrumentation is spike-only: revert it before commit,
  no src diff in this phase. No golden. Exit: the verdict is recorded and
  selects R1.1a vs R1.1b; no src change lands; suite unaffected by this phase.
- **Phase 2 — V2 fix + G1/G2r goldens.** Implement R1.1 at the site Phase 1
  chose: a bare ctor grounds at the caller's own header, never borrowing another
  module's mint. Add G1 (`pb2` → `1\n2`) and G2r (eager-mirror → `1\n2`, primary
  provenance pin) to `tests/phase7b_slice9.rs`, plus
  `bare_ctor_operand_provenance_is_callers_own_header_not_a_borrowed_mint`.
  Matcher untouched (R-NFR2). Exit: G1 + G2r green; #10 and S5 tier-1 unchanged;
  suite +2 green (known-flaky test still not yet re-pinned — that's Phase 3).
- **Phase 3 — V3 fix + G2 golden + flaky-test re-pin.** Implement R2.1: widen
  `instantiation_symbol`'s `Type::Struct`/`Type::Enum` fall-through arm
  (`src/ast.rs:2886`) to render the id already carried by the matched variant
  (no lookup, no signature change — see R2.1), so the shared bound word's two
  groundings mint distinct symbols and lowering's dedup
  (`src/ir/driver.rs:350-373`) keeps both. Add G2 (`mk` variant → `1\n2`, no
  ratio assertion — R-NFR3) plus
  `instantiation_symbol_same_rendered_name_different_struct_ids_mints_distinct_symbols`.
  **Re-pin** `tests/phase7b_slice4.rs`'s
  `same_named_ctor_mk_ambiguity_resolves_but_impl_dispatch_still_cross_picks`
  (same shape as G2, different trait/text) to deterministic `1\n2` and rename
  it off its dead pre-fix criterion, cross-referencing G2; also rewrite its
  comments (`tests/phase7b_slice4.rs:423-425` and `:485-488`), which narrate
  the falsified matcher-blindness story, to the corrected attribution (operand
  provenance + `instantiation_symbol` injectivity, per the mechanism section).
  Exit: G2 and the
  re-pinned slice4 test both print `1\n2` deterministically; the pre-fix flip
  noted only in scratch; the known-flaky pin is deterministically green from
  here on (running total tracked per phase, not a fixed absolute).
- **Phase 4 — D3 determination + G3/G4 goldens.** Build the G4 fixture against
  the Phase-2/3 fix first, observe which of R3's two outcomes actually happens,
  then add G4 (`third_module_mono_caller_is_not_silently_cross_picked` →
  `build_error` pinning the measured text) accordingly. Add G3 (regression pin
  for the existing declaration-time duplicate error text — R5, exact text and
  exit code measured against the fixture, not re-derived) plus
  `check_impl_decls_duplicate_blanket_impl_across_modules_still_errors`.
  Confirm no new machinery in `find_bound_impl` (R4). Exit: G4 and G3 green;
  suite green.
- **Phase 5 — roadmap correction + growth-structure re-check + final gate.**
  Apply R7's five edit targets (`docs/roadmap/P7b-higher-kinded-types.md:221-238`
  two sentences + stale anchor; `docs/roadmap/P7b/slice5-spec.md:776-778` and
  `:783`). Re-run CLAUDE.md split signals against every file S9 touched (R6);
  record the verdict (expected: split still deferred). Final green gate; confirm
  all S9 goldens + #10 + S5 tier-1 pass and no edit landed in the matcher
  (R-NFR2).

## Phases (JSON)

```json
{
  "baseline": { "head": "600bc1b", "binaries": 82, "tests": 3150, "passing": 3149, "knownFlaky": 1, "failed": 0 },
  "phases": [
    {
      "id": 1,
      "name": "Pin the missing candidate (V2 diagnosis gate)",
      "requirements": ["REQ-1"],
      "changes": [
        "Instrument the bare-ctor candidate path (struct_generated_sigs src/check/declarations.rs:1824; single-candidate arm src/check/terms.rs:932; tier path src/check/terms.rs:968-991) on pb2",
        "Record verdict: is a's Widget[i64] mint absent (never minted, bare/inferred) or present-but-filtered from the candidate scan",
        "Revert all instrumentation before commit; append verdict as a new section to docs/roadmap/P7b/slice9-probes.md"
      ],
      "goldens": [],
      "units": [],
      "exit": "REQ-1 verdict recorded in slice9-probes.md, selecting R1.1a (registration/application) vs R1.1b (operand normalization); no src diff lands this phase",
      "notes": "Blocking gate: Phase 2 fix site depends on this verdict (OQ-1). Matcher untouched (R-NFR2). Baseline's known-flaky test (tests/phase7b_slice4.rs:490) is unrelated to this phase; do not chase it."
    },
    {
      "id": 2,
      "name": "V2 fix (operand provenance) + G1/G2r goldens",
      "requirements": ["REQ-2", "REQ-8", "REQ-10"],
      "changes": [
        "Implement R1.1 at the Phase-1-chosen site: bare ctor grounds at caller's own resolved header (src/parser.rs:7101 module-scoped resolution precedent); never substitute another module's eager mint",
        "Matcher and find_bound_impl scan untouched (R-NFR2/R4)"
      ],
      "goldens": [
        "tests/phase7b_slice9.rs: cross_module_same_shaped_impls_dispatch_each_callers_own_impl (G1, pb2) -> 1\\n2",
        "tests/phase7b_slice9.rs: cross_module_same_shaped_impls_eager_minter_wins_regardless_of_caller (G2r, primary provenance pin) -> 1\\n2"
      ],
      "units": [
        "bare_ctor_operand_provenance_is_callers_own_header_not_a_borrowed_mint"
      ],
      "exit": "G1 + G2r print 1\\n2 deterministically; #10 stays 1\\n2, S5 tier-1 stays 15\\n25; fmt/clippy/test green; suite +2 (known-flaky test not yet re-pinned, see Phase 3)",
      "notes": "V2 deterministic pb2 2\\n2 -> 1\\n2. Fix site per REQ-1 (R1.1a or R1.1b)."
    },
    {
      "id": 3,
      "name": "V3 fix (instantiation_symbol widening) + G2 golden + flaky-test re-pin",
      "requirements": ["REQ-3", "REQ-4", "REQ-5", "REQ-10"],
      "changes": [
        "R2.1: widen instantiation_symbol's Type::Struct/Type::Enum fall-through arm (src/ast.rs:2886) to render the StructId/EnumId already carried by the matched Type variant (src/ast.rs:2991-2992) -- no lookup (not struct_instantiation_of/enum_instantiation_of, that is the matcher's own subsystem), no signature change, matching the existing CtorImage arm's GenericId-render approach (:2885; the `GenericId` struct itself is :2521) without importing its lookup mechanism",
        "No change to trait_calls, builtin_overloads, or any IR/lowering file (R-NFR1); the dedup loop at src/ir/driver.rs:350-373 is unmodified and simply stops colliding",
        "Re-pin tests/phase7b_slice4.rs:490 (same_named_ctor_mk_ambiguity_resolves_but_impl_dispatch_still_cross_picks, same shape as G2, different trait/text) from 1\\n1 to deterministic 1\\n2; rename off its dead pre-fix criterion; also rewrite its comments (tests/phase7b_slice4.rs:423-425 and :485-488, currently narrating the falsified find_bound_impl-blindness story) to the corrected attribution (operand provenance + instantiation_symbol injectivity)"
      ],
      "goldens": [
        "tests/phase7b_slice9.rs: cross_module_same_shaped_impls_via_named_instantiation_dispatch_each_callers_own_impl (G2, mk variant) -> deterministic 1\\n2 (NEVER assert a run-count ratio, R-NFR3)"
      ],
      "units": [
        "instantiation_symbol_same_rendered_name_different_struct_ids_mints_distinct_symbols"
      ],
      "exit": "G2 prints 1\\n2 deterministically; re-pinned slice4 test prints 1\\n2 deterministically; pre-fix flip noted only in scratch, not committed; suite fully 3150+N/0 from here on",
      "notes": "V3 nondeterministic 1\\n1/2\\n2 -> deterministic 1\\n2. One mechanism, one fix site (OQ-2 resolved, R2.2 withdrawn). RISK-1: if the widening reds unrelated goldens, stop and escalate per R-NFR1 -- no map-side fallback."
    },
    {
      "id": 4,
      "name": "D3 determination + G3/G4 goldens",
      "requirements": ["REQ-6", "REQ-7", "REQ-8"],
      "changes": [
        "Build the G4 fixture (c.sth wildcard-importing a and b, bare Widget size call) against the Phase-2/3 fix; observe whether c's call surfaces a 2-candidate select_overload collision (outcome a) or grounds against no visible header (outcome b, consistent with probes P5a-ii)",
        "No new visibility/tier machinery in find_bound_impl (R4); declaration-time duplicate check untouched (src/check/declarations.rs:544)"
      ],
      "goldens": [
        "tests/phase7b_slice9.rs: duplicate_blanket_impl_across_modules_is_a_declared_error (G3) -> pins measured 'error: duplicate `impl:` for `'T` (line 3, col 1); first declared at line 3, col 1' text, exit 1",
        "tests/phase7b_slice9.rs: third_module_mono_caller_is_not_silently_cross_picked (G4) -> build_error pinning whichever of R3's outcomes (a)/(b) the fixture actually produces, measured not re-derived"
      ],
      "units": [
        "check_impl_decls_duplicate_blanket_impl_across_modules_still_errors"
      ],
      "exit": "G3 + G4 green; G4 message pinned against actual fixture output (not spec prose); find_bound_impl gained no new machinery; suite green",
      "notes": "OQ-3: build first, pin second. Do not assume outcome (a)."
    },
    {
      "id": 5,
      "name": "Roadmap correction + growth re-check + final gate",
      "requirements": ["REQ-9", "REQ-10"],
      "changes": [
        "R7 target 1: docs/roadmap/P7b-higher-kinded-types.md:221-238, mechanism sentence one ('no module-identity check...') -> the adjudicated two-defect story; state the matcher is sound",
        "R7 target 2: same roadmap entry, :232-234, mechanism sentence two ('find_bound_impl's target-pattern matching itself being blind...') -> replace or remove, equally falsified",
        "R7 target 3: fix stale anchor poly.rs:8218 -> poly.rs:8235 at both sites (roadmap entry and slice5-spec.md:776)",
        "R7 target 4: docs/roadmap/P7b/slice5-spec.md:776-778 ('pb2's actual collision is in find_bound_impl's trait-impl target matching...') -> mark corrected (V1/V2), keep historical marker structure",
        "R7 target 5: docs/roadmap/P7b/slice5-spec.md:783 ('silent 1 1 at exit 0') -> mark corrected to nondeterministic 1\\n1/2\\n2",
        "Reword roadmap exit item 3 (targets 1-2 area): declaration-time duplicate check covers the constructible shape; third-module caller per R3's Phase-4 determination (whichever outcome landed). No history narration in the roadmap's own current-design prose.",
        "R6: re-run CLAUDE.md split signals against every file S9 touched (poly.rs untouched this slice except possibly none; ast.rs, terms.rs, declarations.rs, tests/); record verdict (expected: split still deferred)"
      ],
      "goldens": [],
      "units": [],
      "exit": "All five R7 targets corrected; growth-signal verdict recorded; final fmt/clippy/test green; all S9 goldens + #10 + S5 tier-1 pass; no edit landed in match_impl_target/find_bound_impl scan (R-NFR2)",
      "notes": "Documentation + verification phase; no new behaviour."
    }
  ]
}
```
