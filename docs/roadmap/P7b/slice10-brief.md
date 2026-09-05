# P7b.S10 brief — header-level export ambiguity for the third-module bare caller

- Date: 2026-09-05. Base: `a9eca84` (`cd44b1c` P7b.S9 merged to `main`, plus
  this slice's own recon-round docs; suite 3175/0, unaffected by a docs-only
  commit).
- Carved out of S9's Phase-4 determination (its Residual) and recorded in the
  roadmap's S9 entry Exit clause as future work.
- Sources: recon probe round [slice10-probes](./slice10-probes.md) (verbatim,
  P1–P8 + spike, plus a dated corrections appendix from the
  pre-implementation review round), fixtures and measured behaviour
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

## Adjudicated mechanism (probe round, P6 spike; qualified-spelling finding from
the pre-implementation review round)

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
  in `bare_generated_word_own_module_grounding` (`src/check/terms.rs:1507-1509`
  — the `find_struct(header_name, caller_module)` → `None` arm): caller has no
  own header, one env candidate exists, it is used unchanged.
- The S5 tier policy cannot see the latent ambiguity: with one env candidate
  `select_overload`/`tier_pick` never error (lone-survivor ruling,
  `src/check/builtins.rs:118-127`), and with two env candidates the existing
  accepted-ambiguity error already fires (P2) — naming no modules.
- A *qualified* type spelling in a signature (`a::Widget[i64]`) parse-time-mints
  exactly the same way an unqualified one does: it is a **second eager mint**,
  not a term-position resolution mechanism. A fixture with `b` eager and `c`
  writing `a::Widget[i64]` in its own signature reaches the multi-candidate
  arm and hits the pre-existing, unchanged 2-candidate error — not a cure.

## Working ruling R1 (the policy, reachability-scoped)

**Header-level accepted ambiguity, scoped to the caller's own import set**
(its own imports and selective imports, one hop — not the whole-program file
closure). Once the sole env candidate has survived both the
instantiating-module-is-foreign check and the candidate-identity check (R2's
guard-ordering requirement), the grounding raises a **located compile-time
ambiguity error** at the call site naming: the surface name, the relevant
declaring module(s), and the call span — with remedy pointers — **unless one
of four exemptions holds**. The silent pick becomes an error; the minter
stops deciding (GA/GB both error, regardless of which module spells eagerly).

Reachability-scoping is load-bearing, not a simplification: a program-wide
count would flag GH (`overfire`) — a header some unrelated, unimported module
(`z`) happens to declare — even though `app` never imports `z` and the program
resolves unambiguously today (`7`). This was measured during the
pre-implementation review round, not assumed. A round-2 review measured two
further gaps this revision closes: reachable-header-count alone is not
enough (GM), and the explicit-resolution exemption must resolve through a
re-exporting hub before comparing (GL).

Exemptions (the error must NOT fire when):

- `m` declares its own header — S9's R1.1a grounds the caller's own mint
  (the caller-owns tier, untouched);
- **[both halves required]** at most one same-named header is reachable
  through `m`'s own import set, **and** the sole candidate's actual
  **declaring** module (not its instantiating module — R2) is itself among
  that reachable set — the single-lib compat shape (GD) and the
  unimported-sibling shape (GH) both stay legal because the sole candidate's
  own declaring module is reachable; a header that is reachable but never
  mints does **not** entitle the caller to silently borrow a *different*,
  unreachable module's instantiation instead (GM, a round-2 case: `app`
  imports only `lib`, which declares its own header but never mints; the
  sole instantiation belongs to `z`, never imported by `app` at all — today
  silently prints `9`, `z`'s value; the tightened exemption no longer covers
  this);
- the call reaches the **multi-candidate** arm (≥2 env candidates) — the
  existing S5 `select_overload` path governs there, including tier-2 pinning
  for declared type imports (GJ, P5i2), byte-identical;
- `m` performs an **explicit resolution** — a selective import naming a
  target module for this surface name, that target **resolved through any
  hub re-export chain first** (R2), lands on the sole candidate's actual
  **declaring** module (GI, `sel1`: the sole-minter case; GL, `hub`, a
  round-2 case: `h` re-exports `a`'s `Widget` under its own
  `export: Widget ;`, and `c` selectively imports `Widget` from `h` — `c`'s
  raw selective value is `h`, not `a`; the match must resolve through `h`'s
  own chain to `a` first, or this fixture wrongly errors a program that
  resolves correctly today). The match requirement is load-bearing: a
  selective import names what the caller *asked for*, not what they *get* —
  when the two disagree (GK, a case built during the pre-implementation
  review round), the exemption must not apply, or the exact
  silent-mis-dispatch defect S10 exists to close survives, merely hidden
  behind a selective import that looks like it resolved something (GK is
  curable — R5's two-part remedy 2). A blanket "selective import present ⇒
  exempt" reading, without the match-against-the-declaring-module test and
  without resolving hub re-exports first, is unsound.

## Working ruling R2 (where the check lives, guard ordering, and what data it reads)

**Guard ordering.** The check must run only after the sole candidate has
survived, in order, the `owning_module == caller_module` check (`:1504`) and
the candidate-identity check (`generated_word_entry` + `key`/`symbol` match,
`:1531-1537`). Today's code returns `Ok(None)` immediately at the
`find_struct(header_name, caller_module)` → `None` fall-through (`:1507-1509`,
when the caller has no own header) — *before* it ever reaches the
candidate-identity check, since that check today sits gated behind an own
header being found. The implementing phase restructures this control flow so
a headerless caller still passes through the candidate-identity check first;
emitting directly at `:1507-1509` without that restructuring would newly
reject the "ordinary user word whose output happens to be another module's
instantiation" shape `:1531-1537` protects. The `:1492-1497` "the order is
free" comment is updated to match, since it no longer holds once an arm can
return `Err(...)`.

**Declaring vs. instantiating module.** `struct_instantiation_of` returns the
candidate's *instantiating* module (where the concrete application syntax was
written, `ast.rs:2598`'s `Generic::module` doc comment) — not necessarily the
same as the header's *declaring* module (`guard.structs[gi].module`,
`GenericStructDecl.module`). R1's exemption 2 and exemption 4 both compare
against the declaring module. Every existing golden's candidate happens to be
instantiated inside its own declaring module, so this distinction changes no
golden's outcome; it matters for GL, where the raw selective target is
neither.

**Hub resolution (GL).** A caller's raw `ModuleInfo.selective` value may name
a re-exporting hub with no header of its own. The implementing phase resolves
it through the same hop-walk the checker already runs for a type reference in
an effect signature (`resolve_type_export_origins`/`walk_type_export_origin`,
`driver.rs:364`/`:407`) before comparing against the declaring module —
re-running an equivalent walk at check time, or threading the already-computed
`type_origin` table through, whichever is simpler; the ingredients either way
are already reachable from `Ctx`.

**Naming with no qualifier (GM).** When the sole candidate's declaring module
has no qualifier in `m`'s own imports at all (never imported, GM), the
message names it structurally rather than fabricating one — the same
fallback the existing `drop`-visibility diagnostic already uses for exactly
this gap (`word_families.rs:1319-1340`).

**Everything else.** Caller span, the foreign candidate's owning module
(already computed), header provenance from `ctx.generics().structs` (complete
at env-build time per P6). The caller's own import set comes from
`ctx.modules` (`ModuleInfo.imports` for reachability, `ModuleInfo.selective`
for the hub-resolved explicit-resolution match test); the check never fires
when `ctx.modules` is `None` (same discipline as the existing D1 drop gate).
NOT at `select_overload`/`tier_pick` (cannot see 1-candidate shapes;
lone-survivor ruling), NOT at env build (no call site to locate an error at),
NOT in the matcher (`find_bound_impl`/`match_impl_target` — S9's R-NFR2
carried over; the dispatched `size` member call keeps routing through
`resolve_mono_member_call` → `find_bound_impl` exactly as today whenever
grounding succeeds).

## Working ruling R3 (diagnostics)

A NEW located message (house style: `error: \`Widget\` in \`try\` (line N,
col M) ...` + `note:` remedy line), naming the relevant declaring module(s)
by the caller's own qualifier — except when the sole candidate's declaring
module has no caller-side qualifier at all (GM), where it is named
structurally instead (R2). The existing 2-candidate
`no_overload_matches_error` text is **not** churned (diagnostics are
behaviour; GC pins it unchanged).

## Working ruling R4 (module naming and ordering)

No canonical, caller-independent module-name registry exists in the checker
(`ModuleInfo` has no `name` field) — every existing diagnostic that names a
foreign module uses the **caller's own qualifier** for it. S10 follows the
same precedent, and sorts the collected qualifiers **lexicographically**
before joining them (not registry/import/mint order — `p1-a-b` vs `p1-b-a`
must render identically, and GB must render identically to GA regardless of
which module minted).

## Working ruling R5 (the exact wording)

The note lists two remedies, the second now a **two-part** cure (round-2
correction): (1) declare your own header + impl; (2) selectively import the
module whose instantiation you want — curative **alone** only when it
matches the sole minter (GI, `sel1`), or tier-2 pins it at the
multi-candidate arm (GJ, `p5i2`). When neither holds (GK's shape — the
selected module is reachable but never itself mints), selective import alone
does **not** cure; the caller must **additionally** write an intermediate
word whose own signature spells the concrete type (e.g.
`: mk ( i64 -- Widget[i64] ) Widget ;`) and call that instead of constructing
bare inline — this makes the caller's own module the *instantiating* module,
sidestepping the shared-env borrow entirely rather than merely disambiguating
within it (measured to cure GK's exact shape: `rem2sig`, a round-2 case,
prints `1`, exit 0). Qualified type spelling is dropped from the note
entirely (it never cures; see the mechanism note above).

## Constraints (all measured, all must hold post-change)

1. GD single-lib compat (`p3a-single-lib-private`): one header program-wide →
   unchanged `7`, exit 0.
2. GH unimported-sibling compat (`overfire`): two headers program-wide, only
   one reachable → unchanged `7`, exit 0 — the reachability-scoping witness.
3. GE/GF single-reachable-header compat (`p5g2`, `p7c3`): unchanged `1` — the
   selective import / qualified signature present in these fixtures is
   inert (only one header exists to compete with).
4. GI matching selective import (`sel1`): unchanged `1` — exemption 4 fires
   (the selected module is the sole minter).
5. GJ both-eager selective pinning (`p5i2`): unchanged `1` — multi-candidate
   arm, existing tier-2 pinning untouched.
6. GG type-position naming (`p4-c-annotates`): unchanged `unknown type` error
   — S10 governs term-position bare ctor calls only.
7. GC both-eager 2-candidate error (`p2-both-eager`): byte-identical.
8. GL hub re-export (`hub`): a re-exporting hub with no header of its own
   sits between `c`'s selective import and the true declaring module;
   unchanged `1`, exit 0 — the exemption-4 match resolves through the hub
   chain first (round-2 correction).
9. GM unreachable minter (`under`): only one header (`lib`'s) is reachable,
   but the sole minted instantiation belongs to `z`, never imported at all
   — before: silently prints `9`, exit 0; after: a located ambiguity error,
   named structurally since `app` has no qualifier for `z` (round-2
   correction, the tightened exemption 2).
10. GK mismatched selective import (new fixture): today silently prints `2`
    (the caller selected `a`, but `b` is the sole minter) — after this ruling,
    errors identically to GA/GB. This is the soundness-critical case: exemption
    4 without the match test would leave this silently wrong. Curable via R5's
    two-part remedy 2 — additionally spelling the type in an own-signature
    intermediate word (`rem2sig`) sidesteps the borrow entirely.
11. S9's own goldens (G1/G1a–G1f, G2, G2r, G3) and #10/S5 tier-1: untouched.
    S9's own G4 (`third_module_bare_caller_dispatches_the_single_shared_env_instantiation`)
    is the **one exception** — it is the exact inverse of GA/GB and is retired
    or rewritten in Phase 1.
12. R-NFR1 (no IR/lowering edits) and R-NFR2 (matcher untouched) carried over
    from S9. R-NFR3 (no ratio assertions) for the new goldens.
13. Out of scope, recorded as pre-existing warts: `export: Widget[i64]` parse
    error and the R18 gate's unsatisfiable instantiation remedy (P5a/P5f);
    qualified ctor terms (`a::Widget` → unknown word, P7b2); the concrete-type
    same-name collision dimension. None may be assumed as workarounds; none are
    fixed by S10.

## Open questions — resolved before /implement

1. Error wording: new message as drafted in the spec, or extend
   `no_overload_matches_error` for both shapes? **Resolved: new message; no
   churn to the existing 2-candidate text.**
2. Sequencing: implement S10 before the pending ladder slices S6–S8, or
   after? **Resolved by maintainer ruling (interview, 2026-09-05): in
   parallel with P7b.S6**, both branching from base `a9eca84`. Landing rule:
   if S6's own probes demand a check-stage grounding change in `terms.rs`, or
   the two slices' changes interact at merge time, S10's checker change
   lands first and S6 rebases onto it. The roadmap records "S10 ∥ S6", not a
   strict ladder position.
3. Should the error's remedy note mention the future qualified-ctor-term
   syntax (P7b2), or qualified type spelling generally? **Resolved: neither.**
   Measured during the pre-implementation review round: qualified spelling
   never cures the single-candidate shape (it mints a second candidate,
   routing into the pre-existing 2-candidate error instead). Dropped from the
   note entirely, not phrased as "not yet available".

## Phases (sketch — spec finalizes)

- **Phase 1 — policy + goldens.** Implement R1/R2/R3/R4/R5 at the grounding
  check (restructured for guard ordering, R2); add GA, GB, GK, GM (new errors)
  + regression pins GC, GD, GE, GF, GG, GH, GI, GJ, GL to
  `tests/phase7b_slice10.rs`; retire/rewrite S9's G4 golden; units per
  the paper-tests sketches; error text pinned byte-exact; full gate green.
- **Phase 2 — roadmap + growth re-check + final gate.** Both stale roadmap
  sentences in the S9 entry (the dispatch-mechanism sentence and the trailing
  Residual sentence) updated to "closed by P7b.S10" + a new S10 entry
  (current-design prose, no history narration, recording S10 ∥ S6 sequencing);
  growth signals re-run on every touched file; final full gate ×2.

## Success criteria

GA/GB/GK/GM error deterministically where applicable (byte-exact, both import
orders and both minter placements, the mismatched-selective-import case, and
the unreachable-minter case); GD/GE/GF/GG/GH/GI/GJ/GL/GC byte-identical to
today; S9's G4 retired/rewritten, every other S9 golden +
#10 + S5 tier-1 green; suite fully green; the roadmap's S9 Exit clause no
longer describes an open hole.
