# P7b.S9 — module-aware trait-impl matching (condensed)

> Implemented reference. The delivery plan (numbered requirements, risk tables,
> codebase map, phased steps, phases JSON) is retired; the code and the
> **Implementation** section below supersede it. The frozen companion docs stay
> as-is: [slice9-brief](./slice9-brief.md) (recon), [slice9-probes](./slice9-probes.md)
> (verbatim probe log + Phase-1 verdict + errata) and
> [slice9-paper-tests](./slice9-paper-tests.md) (fixture text). Branch point
> `600bc1b`.

## Why

S9 is the post-ship correction that closed S5. S5's tier policy governs
`env`/`select_overload` ctor-*construction* selection only; the cross-module
mis-dispatch it was chasing did not live in the trait-impl matcher at all. The
probe round (V1/F1/F3) proved the matcher is **sound**:
`match_impl_target_rec`'s `Generic` arm already compares header identity
`(idx, module)` and resolves per-module correctly in every run. The roadmap's
S9 entry ("no module-identity check anywhere in `match_impl_target` /
`select_most_specific`") and slice5-spec's "collision is in `find_bound_impl`"
were both **false**. The real fault was two defects, both off the matcher:

- **V2 — operand provenance** (deterministic `pb2` prints `2\n2`). A bare,
  un-annotated ctor call silently borrowed an unrelated module's eagerly-minted
  instantiation. In `pb2` only `b`'s `usesize` spells `Widget[i64]`, so exactly
  one `Widget[i64]` mint existed program-wide (provenance `gi=1, module=4` =
  b's header); `a::run`'s bare `Widget` ctor applied *that* mint, so both
  callers dispatched with b's provenance and only b's impl matched. The S5 tier
  policy never fired: only one candidate was visible at a's call site.
  Phase-1 verdict (recorded in slice9-probes): **absent-mint** — a's own
  `Widget[i64]` was never minted (a's construction is bare/inferred), so the
  fix belongs at registration/application (R1.1a), not operand normalization.

- **V3 — monomorphization identity** (non-deterministic `mk` variant, `1\n1` or
  `2\n2` per rebuild). Not a shared-map race: `CallInst.trait_calls` is
  per-instantiation. The collapse was one stage later, in lowering's
  instantiation dedup (`src/ir/driver.rs:350-373`), a `HashSet<String>` keyed on
  `instantiation_symbol`. Two groundings of a shared bound word (`sized`) had
  distinct substitutions (`Struct(StructId(2))` vs `Struct(StructId(3))`) but
  `instantiation_symbol`'s rendered-name fall-through arm
  (`other => other.name().to_string()`) was non-injective on them: both minted
  `sooth_mono_sized__m2__t0_Widget_i64_`, so the dedup `HashSet` kept only the
  first `CallInst` reached under randomized `HashMap` iteration order,
  discarding the other grounding's entire `CallInst` (its `trait_calls` map
  included). One built binary reruns deterministically; only rebuilds flip.

(The earlier reading of V3 as `trait_calls.insert(...)` "last-writer-wins across
groundings" was wrong — that call writes a per-instantiation map. See the
slice9-probes errata. The withdrawn R2.2 proposed re-keying `trait_calls` by a
grounding-aware key: inert (it never collides) and out of bounds (lowering-side).)

## What shipped

**V2 fix (R1.1a).** A bare ctor application now grounds at the **caller's own
resolved header** — the same module-scoped resolution the parser already applies
to type positions — minting its own instantiation when none exists, and never
substituting another module's eager mint. The grounding covers the struct's
whole generated pair (ctor + destructure) and re-derives the grounded `Overload`
(signature, lowering symbol, module) from the caller's own minted decl via
`struct_generated_sigs_of` (factored out of `struct_generated_sigs` so env
registration and this path share one rule), read through
`Ctx::with_struct_decl_or_generic` so both a pending mid-word mint and one
already flushed by `check`'s per-word bracket are visible. `check_field_projection`
reads its receiver (struct and variant) through the same accessor. A caller
header that cannot be applied to the call's arguments — wrong arity/length, or a
kind mismatch (an HKT header grounded at a Star-kinded borrowed argument) — is a
located error at the call site, never a silent fall-back to the borrowed mint.
The matcher is untouched: with identity-correct operands, distinct headers have
distinct `(idx, module)` and the right pattern matches.

**Layout name-keys (R2.4, R-NFR1's sole sanctioned lowering-side exception).**
Because two live `StructDecl`s now share one `type_instantiation_name` spelling,
`ir/layout.rs`'s two module-blind name-key layers were made module-unique:
(i) the generated-word registry (`swords`), read at the call term's own module;
(ii) the emitted type symbol (`StructLayout`/`EnumLayout` name), qualified
`{name}__m{module}` for duplicated names only — so a program with no two
same-named decls emits byte-identical IL. Naming only: no dispatch, visibility,
or tier logic.

**V3 fix (R2.1).** `instantiation_symbol` (`src/ast.rs:2869`) IS the mono key
(one source of truth for the checker's call-site table and `IrFunc.name`). Its
`Type::Struct`/`Type::Enum` fall-through arm now renders the carried
`StructId`/`EnumId` inner index (`s{id}_{name}` / `e{id}_{name}`,
`src/ast.rs:2894-2895`), the same way the `CtorImage` arm already renders its
`GenericId`. A `StructId`/`EnumId` is globally unique across modules on its own
(the `Module::structs`/`enums` registries are whole-program-assembled), so no
module component and no lookup are needed; the signature is unchanged and all
call sites keep compiling. Two groundings now mint distinct symbols, so the
dedup loop keeps both `CallInst`s (both `trait_calls` maps) and each compiles
its own body. The dedup loop and every `trait_calls` consumer are untouched:
they simply stop colliding.

**Phase-4 determination (D3, measured 2026-09-04).** The third-module bare-caller
case resolved to a **third outcome** neither R3 candidate anticipated. A third
module `c` declaring no `Widget` header of its own, wildcard-importing `a` and
`b`: build succeeds, binary deterministically prints `2` (b's constant), 8/8
rebuild cycles, both import orders. Mechanism: R1.1a's own-header grounding has
nothing to ground at (`c` declares no header); neither `a`'s nor `b`'s own
`Widget[i64]` is exported (exporting it is itself a "names private type" error);
the only `Widget[i64]` visible to `c`'s env lookup is the single instantiation
minted into the shared whole-program env (b's, spelled in `usesize`'s signature).
With exactly one candidate, the pre-existing single-candidate arm takes it
silently. `c`'s bare `size` (a trait member call on a concrete operand) routes
through `resolve_mono_member_call` → `find_bound_impl`, which is handed a single
unambiguous target (removing b's impl surfaces `find_bound_impl`'s own "no impl"
error). The *ambiguity* path (`select_overload`'s 2-candidate collision) never
fires because only one instantiation is ever visible. `find_bound_impl` gained
no new machinery — confirmed by measurement.

**Cross-module blanket-impl ambiguity (R4).** A single cross-module blanket
`impl: Sized for 'T` is already placement-illegal (`check_impl_decls`'
must-live-in-declaring-module rule, `src/check/declarations.rs`). Two such impls
surface as the **duplicate** error — but only because the module-blind duplicate
scan runs before the placement loop and never reads `imp.module`; a bare
`PolyType::Var` carries no header identity to tell them apart. That scan's own
coverage is the same-module duplicate shape; the cross-module pair reaches it by
loop order, not by design. `ImplDecl.module` stays read only by the placement
rule. No new dispatch-time visibility or tier machinery was added anywhere.

## Load-bearing rulings

- **R1.1a** — bare ctors ground at the caller's own header (absent-mint fix at
  registration/application), never borrowing another module's eager mint; the
  matcher stays untouched.
- **R2.1** — `instantiation_symbol` is injective on grounding identity: same
  rendered name + different `StructId`/`EnumId` ⇒ distinct symbols. This one
  fix-site delivers determinism; there is no map-side fallback (R2.2 withdrawn).
- **R2.4** — `instantiation_symbol` (word mono key) and the generated-word /
  emitted-type-symbol registry (`ir/layout.rs`) are two distinct name-key layers.
  R2.1 covers the first; the module-unique layout keys cover the second.
- **R-NFR1 (as amended in Phase 3)** — the V3 mechanism fix is check-stage only
  (`instantiation_symbol`); the dedup loop, `trait_calls`, and `builtin_overloads`
  are untouched. The **sole** sanctioned lowering-side exception is Phase 2's
  `ir/layout.rs` name keys (both layers) — nothing else. Any other candidate fix
  needing an IR/lowering edit stops and escalates.
- **R-NFR2** — `match_impl_target`/`..._rec` and `find_bound_impl`'s scan have
  zero behavioural diff (verified: `src/check/poly.rs` had a +1/-1 test-string
  change only).

## Goldens (identities and behaviour)

All in `tests/phase7b_slice9.rs` unless noted. Fixture text is preserved in
[slice9-paper-tests](./slice9-paper-tests.md).

| Golden | Test name | Behaviour |
| --- | --- | --- |
| G1 | `cross_module_same_shaped_impls_dispatch_each_callers_own_impl` (verbatim `pb2`) | `2\n2` → `1\n2` (V2) |
| G2 | `cross_module_same_shaped_impls_via_named_instantiation_dispatch_each_callers_own_impl` (`mk` variant) | nondeterministic `1\n1`/`2\n2` → deterministic `1\n2` (V3); never asserts a run-count ratio (R-NFR3) |
| G2r | `cross_module_same_shaped_impls_eager_minter_wins_regardless_of_caller` (a eager / b bare — primary provenance pin) | `1\n1` → `1\n2`; **lands in Phase 3**, not Phase 2 — post-V2 it hits the V3 symbol collision until R2.1 |
| G3 | `duplicate_blanket_impl_across_modules_is_a_declared_error` | regression pin on the declaration-time duplicate error (exit 1) |
| G4 | `third_module_bare_caller_dispatches_the_single_shared_env_instantiation` (renamed from the placeholder `third_module_mono_caller_is_not_silently_cross_picked`) | deterministic `2`, exit 0, both import orders — the measured third outcome |
| G5 | #10 `same_named_ctors_in_two_modules_dispatch_distinct_impls` (`tests/phase7b_slice2.rs`); S5 tier-1 `cross_module_same_shaped_ctor_dispatches_callers_own_impl` (`tests/phase7b_slice5.rs`) | unchanged: `1\n2`; `15\n25` |

Phase 2 also added G1a–G1f (every same-module site grounds at its own header;
field projection; arity/kind mismatch diagnostics; the two-module collision
shape for destructure and bundle-pack) plus the `bare_*` units. The pre-existing
flaky pin `same_named_ctor_mk_ambiguity_resolves_but_impl_dispatch_still_cross_picks`
(`tests/phase7b_slice4.rs`, same shape as G2, different trait/text) was re-pinned
to deterministic `1\n2` in Phase 3 and renamed off its dead pre-fix criterion.

Key unit: `instantiation_symbol_same_rendered_name_different_struct_ids_mints_distinct_symbols`
(`src/ast.rs`).

## Residual / known limitation

Two modules each instantiating a same-shaped type via a same-shaped `impl`,
consumed by a third module with no header of its own, resolves **silently and
deterministically** to whichever instantiation happens to be the single one
minted into the shared whole-program env — not to a compile-time ambiguity
error. Closing this is a new cross-module *import-resolution* (export-ambiguity)
policy, not a trait-dispatch fix: R4 forbids the dispatch-time machinery
(`find_bound_impl`) a naive fix would reach for. It is future work needing its
own brief/probes/spec, and is recorded as a roadmap follow-up (see the R7
correction below), not an S9 regression.

## Growth-structure re-check (R6)

Split signals re-run at Phase 5 exit against every file S9 touched: none crossed
the 2-signal refactor threshold. `src/check/poly.rs` (~21k lines) had a +1/-1
test-string change only; its split remains deferred (pre-existing 3/5 signals,
no clean cut, S5 residual) — unchanged by this slice. Every non-test addition
sits beside the code it extends (same-neighbourhood or a direct factoring-out
like `struct_generated_sigs_of`), so no import-divergence or X/Y/Z-mixing signal
newly fired.

## Roadmap corrections applied (R7)

Six edit targets landed in Phase 5:

1–2. `docs/roadmap/P7b-higher-kinded-types.md` — both falsified mechanism
sentences ("no module-identity check…" and "find_bound_impl's target-pattern
matching itself being blind…") replaced with the adjudicated two-defect story;
the matcher stated sound.
3. Stale anchor `poly.rs:8218` → `:8235` at both sites.
4–5. `docs/roadmap/P7b/slice5-spec.md:776-778` and `:783` — the "collision is in
`find_bound_impl`" and "silent `1 1`" claims marked corrected inside their
historical post-ship-correction blockquote (correction marked, record kept).
6. The Phase-4 export-ambiguity residual recorded as a roadmap follow-up.

The roadmap exit criterion was reworded to the landed outcome: the constructible
ambiguity shape is covered by the declaration-time duplicate check (no new
dispatch-time mechanism); a third-module bare caller with no own header
dispatches deterministically on the single shared-env instantiation. It does
**not** imply "add a dispatch-time ambiguity error".

## Implementation

| Area | Commit(s) | Key files |
| --- | --- | --- |
| Phase-1 verdict (R1.0, absent-mint) | `38d90bd` | `docs/roadmap/P7b/slice9-probes.md` |
| V2 fix — caller-owned grounding (R1.1a) + layout name-keys (R2.4) + G1/G1a–G1f | `9b15d80`, `5fd9d28` | `src/check/terms.rs`, `src/check/declarations.rs`, `src/check/word_families.rs`, `src/ir/layout.rs`, `src/ir/func_builder/calls.rs`, `tests/phase7b_slice9.rs` |
| V3 fix — `instantiation_symbol` injective (R2.1) + G2/G2r + slice4 re-pin | `5a99385` | `src/ast.rs`, `src/check/poly.rs` (test string), `src/ir/driver.rs` (test strings), `tests/phase7b_slice4.rs`, `tests/phase7b_slice9.rs` |
| QBE baseline regen for the mono-symbol rendering (RISK-1, rename-only) | `d2e8123` | `tests/qbe_baseline/*.ssa` (34 files, +476/-476, zero structural IL diff) |
| Phase-4 D3 determination + G3/G4 | `59dd87b` | `src/check/declarations.rs` (duplicate-check unit), `tests/phase7b_slice9.rs` |
| Roadmap correction + growth re-check + final gate (R7/R6) | `6c0a0ad` | `docs/roadmap/P7b-higher-kinded-types.md`, `docs/roadmap/P7b/slice5-spec.md`, `tests/phase7b_slice4.rs` |
