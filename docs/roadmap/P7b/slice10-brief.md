# P7b.S10 brief — header-level export ambiguity for the third-module bare caller

- Date: 2026-09-05. Base: `cd44b1c` (P7b.S9 merged to `main`; suite 3175/0).
- Carved out of S9's Phase-4 determination (its Residual) and recorded in the
  roadmap's S9 entry Exit clause as future work.
- Sources: recon probe round [slice10-probes](./slice10-probes.md) (verbatim,
  P1–P8 + spike), fixtures and measured behaviour
  [slice10-paper-tests](./slice10-paper-tests.md). Companion to S9's docs;
  read [slice9-spec](./slice9-spec.md) (condensed reference) for the
  two-defect story and R1.1a/R2.1/R2.4/R-NFR context.

## Problem

S9 fixed operand provenance (V2) and monomorphization identity (V3). Its
Phase-4 determination then measured the remaining third-module shape: modules
`a` and `b` each declare their own same-named generic header
(`Widget['T]`) with their own `impl: Sized for Widget` (constants 1 and 2);
module `c` imports both, declares no `Widget` header, and makes a bare
`Widget` ctor call. Today that call **builds and silently dispatches on
whichever module happens to spell the instantiation eagerly** — probes P1/P8:
with b eager, c prints `2`; with a eager, c prints `1`. The output of `c` is
decided by the internals of a module `c`'s author may never have read: a
silent, semantically load-bearing dependence, exactly the failure class this
roadmap exists to turn into sharp compile errors (CLAUDE.md).

## Adjudicated mechanism (probe round, P6 spike)

- The env is built **once, before any body is checked**
  (`src/check.rs:586`, `struct_generated_sigs(&module.structs)`), from
  **parse-time eager mints only**. A module's own-header grounding (S9's
  R1.1a) mints mid-check inside its own word loop and never precedes another
  module's env build.
- So a headerless caller's bare ctor call sees a candidate list of exactly the
  eagerly-spelled instantiations — in the G4 shape, **one** (b's
  `Widget[i64]@m8`), even though **both headers are already visible** in
  `module.generic_structs` at env-build time (`["Widget@m7","Widget@m8"]`).
- The silent pick happens at the single-candidate arm's grounding fall-through
  (`src/check/terms.rs:1516-1523` region): caller has no own header
  (`find_struct(name, caller_module)` → `None`), one env candidate exists, it
  is used unchanged.
- The S5 tier policy cannot see the latent ambiguity: with one env candidate
  `select_overload`/`tier_pick` never error (lone-survivor ruling,
  `src/check/builtins.rs:118-127`), and with two env candidates the existing
  accepted-ambiguity error already fires (P2) — naming no modules.

## Working ruling R1 (the policy)

**Header-level accepted ambiguity.** When a bare ctor/destructure call in
module `m` reaches the single-candidate arm with exactly one env candidate
whose mint's header belongs to a foreign module, and the generic header
registry holds **≥2 same-named headers declared by ≥2 distinct modules, none
of them `m`**, the grounding raises a **located compile-time ambiguity error**
at the call site naming: the surface name, the ≥2 declaring modules, and the
call span — with remedy pointers. The silent pick becomes an error; the minter
stops deciding (GA/GB both error, regardless of which module spells eagerly).

Exemptions (the error must NOT fire when):

- `m` declares its own header — S9's R1.1a grounds the caller's own mint
  (the caller-owns tier, untouched);
- exactly **one** same-named header exists program-wide — the single-lib
  compat shape (GD) stays legal and silent-by-design;
- the call reaches the **multi-candidate** arm (≥2 env candidates) — the
  existing S5 `select_overload` path governs there, including tier-2 pinning
  for declared type imports (GE/GF, P5i2), byte-identical.

## Working ruling R2 (where the check lives)

At the single-candidate arm's grounding check in `src/check/terms.rs` —
caller span, the foreign candidate, and header provenance from the generic
registry (`ctx.generics()`; complete at env-build time per P6). NOT at
`select_overload`/`tier_pick` (cannot see 1-candidate shapes; lone-survivor
ruling), NOT at env build (no call site to locate an error at), NOT in the
matcher (`find_bound_impl`/`match_impl_target` — S9's R-NFR2 carried over;
the dispatched `size` member call keeps routing through `resolve_mono_member_call`
→ `find_bound_impl` exactly as today whenever grounding succeeds).

## Working ruling R3 (diagnostics)

A NEW located message (house style: `error: \`Widget\` in \`try\` (line N,
col M) ...` + `note:` remedy line). The existing 2-candidate
`no_overload_matches_error` text is **not** churned (diagnostics are
behaviour; GC pins it unchanged). The exact wording is fixed in the spec and
pinned byte-exact; it must name the surface name, the ≥2 declaring modules,
and the call site, and point at the three remedies that exist today.

## Constraints (all measured, all must hold post-change)

1. GD single-lib compat (`p3-single-lib`): one header program-wide → unchanged
   `7`, exit 0.
2. GE selective type import (`p5g2`): unchanged `1` — an explicit resolution;
   tier-2 pinning (P5i2) must not be re-errored.
3. GF qualified type spelling (`p7c3`): unchanged `1`.
4. GG type-position naming (`p4-c-annotates`): unchanged `unknown type` error
   — S10 governs term-position bare ctor calls only.
5. GC both-eager 2-candidate error (`p2-both-eager`): byte-identical.
6. S9's own goldens (G1/G1a–G1f, G2, G2r, G3, G4) and #10/S5 tier-1: untouched
   — the new check sits beside S9's R1.1a grounding and only fires where R1.1a
   falls through today.
7. R-NFR1 (no IR/lowering edits) and R-NFR2 (matcher untouched) carried over
   from S9. R-NFR3 (no ratio assertions) for the new goldens.
8. Out of scope, recorded as pre-existing warts: `export: Widget[i64]` parse
   error and the R18 gate's unsatisfiable instantiation remedy (P5a/P5f);
   qualified ctor terms (`a::Widget` → unknown word, P7b2); the concrete-type
   same-name collision dimension. None may be assumed as workarounds; none are
   fixed by S10.

## Open questions for the user (default answers in parentheses)

1. Error wording: new message as drafted in the spec, or extend
   `no_overload_matches_error` for both shapes? (Default: new message; no
   churn to the existing 2-candidate text.)
2. Sequencing: implement S10 before the pending ladder slices S6–S8, or after?
   (Default: before — it is small, independent, and closes a correctness gap
   in the same area; S6–S8 build on the trait ladder, not on this policy.)
3. Should the error's remedy note mention the future qualified-ctor-term
   syntax (P7b2) as "not yet available"? (Default: no — remedies listed are
   only the three that work today.)

## Phases (sketch — spec-writer finalizes)

- **Phase 1 — policy + goldens.** Implement R1/R2/R3 at the grounding check;
  add GA, GB (+ regression pins GC, GD, GE, GF, GG) to
  `tests/phase7b_slice10.rs`; units per the paper-tests sketches; error text
  pinned byte-exact; full gate green (3175+N/0; the slice4 flake is gone since
  S9 Phase 3 — expect zero failures).
- **Phase 2 — roadmap + growth re-check + final gate.** Roadmap S9 entry's
  Residual sentence updated to "closed by P7b.S10" + a new S10 entry
  (current-design prose, no history narration); growth signals re-run on every
  touched file; final full gate ×2.

## Success criteria

GA/GB error deterministically (byte-exact, N/3 cycles, both import orders and
both minter placements); GD/GF/GE/GG/GC byte-identical to today; all S9
goldens + #10 + S5 tier-1 green; suite fully green; the roadmap's S9 Residual
no longer describes an open hole.
