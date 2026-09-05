# P7b.S10 spec — header-level export ambiguity for the third-module bare caller

> Delivery spec. Companion frozen docs: [slice10-brief](./slice10-brief.md)
> (rulings R1-R5, constraints, open questions), [slice10-paper-tests](./slice10-paper-tests.md)
> (complete fixtures + measured-before columns + unit sketches),
> [slice10-probes](./slice10-probes.md) (verbatim probe log + verdict, plus a dated
> corrections appendix from the pre-implementation review round). Read
> [slice9-spec](./slice9-spec.md) for the two-defect story and the
> R1.1a / R2.4 / R-NFR context this slice sits beside. Base: `a9eca84`
> (P7b.S9 merged to `main` at `cd44b1c`, plus this slice's own recon-round docs;
> suite 3175/0, unaffected by a docs-only commit).

## Why

S9 closed operand provenance (V2, R1.1a) and monomorphization identity (V3,
R2.1) and left one measured hole as its Phase-4 Residual: two modules `a` and
`b` each declare their own same-named generic header (`Widget['T]`) with their
own `impl: Sized for Widget` (constants 1 and 2); a third module `c` imports
both, declares **no** `Widget` header, and makes a **bare** `Widget` ctor call.
Today that call builds and **silently dispatches on whichever module happens to
spell the instantiation eagerly** (probes P1/P8: with `b` eager, `c` prints `2`;
with `a` eager, `c` prints `1`). The output of `c` is decided by internals of a
module `c`'s author may never have read — the silent, semantically load-bearing
dependence this roadmap exists to turn into sharp compile errors (CLAUDE.md).

S9's R1.1a own-header grounding has nothing to ground at (`c` declares no
header); the only `Widget[i64]` visible to `c`'s env lookup is the single
eagerly-minted instantiation, so the pre-existing single-candidate arm takes it
silently. S10 replaces that silent pick with a **located compile-time ambiguity
error**, at the single-candidate grounding fall-through, keyed on the generic
header registry **scoped to the caller's own import closure** (R1) — not on the
trait-impl matcher (R2/R-NFR2 forbid the dispatch-time machinery a naive fix
would reach for). Reachability-scoping is load-bearing, not a simplification: a
program-wide header count would flag a header some unrelated, unimported module
happens to declare, breaking single-lib programs that never see it (GH,
measured during the pre-implementation review round — see below).

## Adjudicated mechanism (probe round, P6 spike — verbatim in slice10-probes)

- The env is built **once, before any body is checked** (`src/check.rs:586`,
  `struct_generated_sigs(&module.structs)`), from parse-time **eager mints
  only**. A module's own-header grounding (R1.1a) mints mid-check inside its own
  word loop and never precedes another module's env build.
- So a headerless caller's bare ctor call sees a candidate list of exactly the
  eagerly-spelled instantiations — in the G4 shape, **one** (`b`'s
  `Widget[i64]@m8`), even though **both** headers are already visible in
  `module.generic_structs` at env-build time (`["Widget@m7","Widget@m8"]`).
- The silent pick happens at the single-candidate arm's grounding fall-through
  in `bare_generated_word_own_module_grounding` (`src/check/terms.rs:1507-1509`
  — the `find_struct(header_name, caller_module)` → `None` arm): caller has no
  own header, one env candidate exists, it is used unchanged (`Ok(None)`,
  falling through to the borrowed mint).
- The S5 tier policy cannot see the latent ambiguity: with one env candidate
  `select_overload`/`tier_pick` never error (lone-survivor ruling,
  `src/check/builtins.rs:118-127`); with two env candidates the existing
  accepted-ambiguity error already fires (P2), naming no modules.
- **Confirmed during the pre-implementation review round:** a *qualified* type
  spelling in a signature (`a::Widget[i64]`) parse-time-mints exactly the same
  way an unqualified one does — it is not a term-position resolution mechanism,
  it is a **second eager mint**. A fixture with `b` eager via its own signature
  and `c` writing `a::Widget[i64]` in *its own* signature reaches the
  **multi-candidate** arm (2 env candidates) and hits the pre-existing,
  unchanged 2-candidate `no_overload_matches_error` — the same error GC pins,
  not a new one, and not a cure. This is why R5's remedy note drops qualified
  spelling entirely rather than listing it as "not yet available" (OQ-3).

## Rulings

### R1 — the policy (reachability-scoped header-level accepted ambiguity)

When a bare ctor/destructure call in module `m` reaches the single-candidate
arm (the fall-through in `bare_generated_word_own_module_grounding`) with
exactly **one** env candidate whose mint's header belongs to a foreign module
`M`, the grounding raises a **located compile-time ambiguity error** at the
call site naming the surface name, the declaring modules, and the call span,
with remedy pointers — **iff** the generic header registry
(`ctx.generics().structs`) holds **≥2 same-named headers declared by ≥2
distinct modules reachable through `m`'s own `import:` declarations**
(`ModuleInfo.imports` ∪ `ModuleInfo.selective` target sets for `m`; see R2),
excluding `m` itself.

Reachability, not program-wide existence, is the count: a header declared by a
module `m` never imports in any form does not count toward the ambiguity,
regardless of how many instantiations exist elsewhere in the program (GH,
`overfire`: two headers exist program-wide, `lib` and `z`; `app` imports only
`lib`, never `z`; only one header is reachable from `app`, so the single-lib
compat exemption applies even though a second header exists somewhere in the
closure — measured `7`, exit 0, unchanged).

**Exemptions — the error must NOT fire when:**

1. `m` declares its own header — S9's R1.1a grounds the caller's own mint (the
   caller-owns tier, untouched);
2. at most **one** same-named header is reachable through `m`'s own imports
   (regardless of how many exist program-wide) — the single-lib compat shape
   (GD) and the unimported-sibling shape (GH) both stay legal and
   silent-by-design;
3. the call reaches the **multi-candidate** arm (≥2 env candidates) — the
   existing S5 `select_overload` path governs there unchanged, including
   tier-2 pinning for declared type imports (GJ, P5i2), byte-identical;
4. `m` performs an **explicit resolution** for this surface name —
   `ModuleInfo.selective` (populated by a `| Widget |` selective-import clause,
   or by a `*` wildcard import's per-export desugaring; see R2) names exactly
   one reachable module for `Widget`, **and that named module is exactly `M`**
   (the actual sole candidate's owning module) — GI (`sel1`).

Exemption 4's match requirement is load-bearing, not decorative:
`ModuleInfo.selective` records what the caller *asked for*, not what the
caller *gets* — the two agree only when the module the caller selected happens
to be the one that eagerly minted. When they disagree (the caller selectively
imports module `X`'s `Widget`, but the sole existing instantiation belongs to a
different reachable module `Y`), the exemption must **not** apply: silently
handing the caller `Y`'s value while they asked for `X`'s is the exact
V2-class defect S10 exists to close, only relabeled behind a selective import
the caller believes already disambiguated it. Measured today (GK, a case
built during the pre-implementation review round specifically to probe this
gap): `c` selectively imports `a`'s `Widget` (`import: self::a | Widget | ;`)
while only `b` ever eagerly mints — today this silently prints `2` (`b`'s
value, contradicting the caller's own selection), exit 0. Under this ruling it
becomes the **same** located error as GA/GB, naming both reachable modules. **A
blanket "selective import is present ⇒ exempt" reading, without the match test,
is unsound and must not be implemented** — it would leave exactly the silent
mis-dispatch this slice exists to close, merely hidden behind a selective
import that looks like it resolved something.

### R2 — where the check lives, and what data it reads

At the single-candidate arm's grounding check in
`bare_generated_word_own_module_grounding` (`src/check/terms.rs:1507-1509`, the
`find_struct(header_name, caller_module)` → `None` arm) — reading:

- the caller's span and module (already in scope);
- the foreign candidate's owning module, `owning_module` (already computed by
  `struct_instantiation_of`, line 1501 — no new lookup needed);
- header provenance from `ctx.generics().structs` (`GenericStructDecl.module`,
  `GenericStructDecl.name`) — the full, whole-program-declared header list;
- the caller's own import closure from `ctx.modules` (`Option<&[ModuleInfo]>`,
  `engine.rs:1133`; `Some` on the whole-program build path, `None` off it —
  e.g. a retained poly word), indexed by `caller_module`:
  `ModuleInfo.imports` (qualifier → target module id, `ast.rs:176`) for
  reachability, and `ModuleInfo.selective` (bare name → target module id,
  `ast.rs:185` — merging explicit `| name |` clauses and `*` wildcard
  per-export desugaring, `driver.rs`'s import-assembly pass) for exemption 4's
  match test.

When `ctx.modules` is `None`, the check does not fire — the same "reads it and
never fires when it is absent" discipline the existing D1 drop gate follows
(`engine.rs:1133`'s doc comment: "the gate reads it and never fires when it is
absent"), since there is no import closure to test reachability against.

**NOT** at `select_overload`/`tier_pick` (cannot see 1-candidate shapes; the
lone-survivor ruling deliberately never errors on one candidate). **NOT** at
env build (`src/check.rs:586`; no call site to locate an error at). **NOT** in
the matcher (`find_bound_impl` / `match_impl_target` — S9's R-NFR2 carried
over: the dispatched `size` member call keeps routing through
`resolve_mono_member_call` → `find_bound_impl` exactly as today whenever
grounding succeeds).

### R3 — diagnostics

A **NEW** located message in house style (`error: \`Widget\` in \`try\` (line N,
col M) is ambiguous: ...` + a `note:` remedy line). It names the surface name,
the declaring modules, and the call site, and points at the remedies that
actually cure the shape (R5). The existing 2-candidate `no_overload_matches_error`
text is **not churned** (diagnostics are behaviour; GC pins it byte-unchanged).

### R4 — module naming and deterministic ordering

No canonical, caller-independent module-name registry exists anywhere in the
checker (`ModuleInfo` carries no `name` field, `ast.rs:175-189`) — every
existing diagnostic that names a foreign module renders the **caller's own
qualifier** for it (`declarations.rs:974`, `:988`, `word_families.rs:1336` are
the precedent: `` module `{qualifier}` `` from the caller's own `import:`
binding). S10 follows the same precedent: for each reachable declaring module,
print the qualifier `m`'s own `ModuleInfo.imports` binds to it. A bare
`import: self::a ;` binds qualifier `a` by default; no renaming/aliasing
syntax exists for `self::` imports (confirmed: no `as`-style grammar in the
parser). If `m` reaches a declaring module only through a wildcard import (no
bound qualifier — `ModuleInfo.imports` has no entry for it, only
`ModuleInfo.selective` does, per-export), fall back to the existing
wildcard-import phrasing precedent (`declarations.rs:969`, "its
wildcard-imported module"). No fixture in this spec exercises the
both-declaring-modules-wildcard-only sub-case, so a distinguishable phrasing
for 2+ unqualified modules is left to the implementing phase — flagged, not
blocking; no golden requires it.

Determinism (REQ-5) follows directly: the message **sorts the collected
qualifier strings lexicographically** before joining them, rather than using
registry order (import-order-dependent — `p1-a-b` vs `p1-b-a` reach the
fall-through with the identical reachable set `{a, b}` regardless of which
import statement came first, so a lexicographic sort names them `a` then `b`
in both orders) or mint order (minter-placement-dependent — the exact
dependence GB exists to falsify).

### R5 — the exact wording (contract; the golden pins the measured bytes)

The new message follows the measure-then-pin discipline: this spec fixes the
wording **contract** below; the implementing phase measures the real rendered
output and pins whatever the formatter actually emits (spacing, the
parenthetical `(line N, col M)` shape, and how module names are joined all
follow the existing `terms.rs` diagnostic helpers). Draft contract:

```
error: `Widget` in `try` (line 3, col 22) is ambiguous: declared in modules `a` and `b`, and `try`'s module declares no `Widget`
  note: declare your own `Widget` header and impl, or selectively import the module whose `Widget` you want (`import: self::a | Widget | ;` after `export: Widget ;`)
```

Contract, byte-exact wording deferred to the golden:

- lead line: surface name in backticks, the enclosing word in backticks, the
  `(line N, col M)` call site, the word "ambiguous", and the declaring module
  names, lexicographically sorted (R4);
- note line: the **two** remedies that actually cure the single-candidate
  shape — (1) declare your own `Widget` header and `impl`; (2) selectively
  import the module whose instantiation you want. Remedy 2 is curative **only**
  when the named module either is the sole eager minter (GI, `sel1`) or both
  modules mint and tier-2 pins your selection (GJ, `p5i2`) — a selective
  import naming a module that is *not* the sole minter does **not** cure (GK)
  and still errors. Qualified type spelling (`a::Widget[i64]`) is **dropped**
  from the note entirely (OQ-3, revised from "not yet available" to simply
  omitted): it is a type-position tool only — no term-position ctor syntax
  exists (`a::Widget` is an unknown word, P7b2) — and using it in a signature
  to try to pin an operand's type just parse-time-mints a *second* candidate,
  routing into the pre-existing, unchanged 2-candidate error (GC's shape)
  rather than resolving anything (confirmed by measurement, not a cure).

## Requirements

- **REQ-1 (policy, R1).** At the single-candidate grounding arm, a bare
  ctor/destructure call in module `m` with exactly one foreign env candidate
  fires the located ambiguity error **iff** the generic registry holds ≥2
  same-named headers from ≥2 distinct modules **reachable through `m`'s own
  imports**, none of them `m`. The four exemptions (own header; ≤1 reachable
  header; multi-candidate arm; matching explicit resolution) hold exactly — in
  particular exemption 4 requires the resolved target to equal the actual sole
  candidate's owning module, not merely that some selective import exists.
  Traces to GA, GB, GD, GH, GC, GI, GK; units
  `ambiguous_foreign_headers_grounding_is_located_error`,
  `single_foreign_header_grounding_still_borrows`, `own_header_still_grounded_first`,
  `reachable_header_count_excludes_unimported_declaring_modules`,
  `matching_selective_import_exempts_the_ambiguity_check`,
  `mismatched_selective_import_does_not_exempt_the_ambiguity_check`.
- **REQ-2 (layer, R2).** The check lives at the `terms.rs:1507-1509` grounding
  fall-through, reading header provenance from `ctx.generics().structs` and the
  caller's import closure from `ctx.modules` (`ModuleInfo.imports`/`.selective`).
  No edit to `select_overload`, `tier_pick`, env build (`check.rs:586`), or the
  matcher (`find_bound_impl`/`match_impl_target`). When `ctx.modules` is
  `None`, the check never fires. Traces to REQ-7 guardrails; unit
  `own_header_still_grounded_first`.
- **REQ-3 (diagnostic, R3/R5).** A new located message naming surface name,
  declaring modules, and call site, with the two-remedy note; the existing
  2-candidate `no_overload_matches_error` text is byte-unchanged. Traces to GA,
  GB, GC, GK; unit `ambiguous_header_error_names_declaring_modules`.
- **REQ-4 (compat pins).** GD/GE/GF/GG/GH/GJ byte-identical to today; GC
  byte-identical; P5i2 tier-2 selective-import pinning untouched (not
  re-errored). Traces to GC/GD/GE/GF/GG/GH/GJ.
- **REQ-5 (determinism and naming, R4).** GA/GB/GK error **identically**
  (byte-exact) regardless of which module is the eager minter and regardless
  of import order — declaring modules are named by the caller's own import
  qualifier (R4) and joined in lexicographic order, not registry/import/mint
  order. No run-count ratio asserted (R-NFR3). Traces to GA, GB, GK.
- **REQ-6 (S9 goldens: one inverts, the rest untouched).** S9's
  `third_module_bare_caller_dispatches_the_single_shared_env_instantiation`
  (`tests/phase7b_slice9.rs`) is the **exact inverse** of GA/GB — it currently
  asserts the silent `2`/`1` outputs this slice replaces with a located error.
  This golden is **retired or rewritten** in Phase 1 (not merely "untouched");
  every other S9 golden — G1/G1a–G1f, G2, G2r, G3, plus #10 and S5 tier-1 —
  stays green and byte-unchanged, since the new check sits beside R1.1a's
  grounding and only fires where R1.1a falls through today, on the specific
  shape S9 documented as its Residual.
- **REQ-7 (guardrails).** R-NFR1 (no IR/lowering edits — and the S9
  `ir/layout.rs` exception does **not** carry into S10; S10 is check-stage
  only), R-NFR2 (matcher untouched), R-NFR3 (no ratio assertions on the new
  goldens). Diagnostics are behaviour: only the NEW message is new; no
  existing diagnostic text churns.
- **REQ-8 (roadmap + growth + gate).** The roadmap's S9 entry's Exit clause
  (`docs/roadmap/P7b-higher-kinded-types.md`, its trailing "Residual: ...
  future work" sentence) updated to state the shape is closed by P7b.S10; a
  new S10 entry in current-design prose (no history narration), recording that
  S10 is implemented **in parallel with P7b.S6** (maintainer ruling,
  2026-09-05; both branch from base `a9eca84`) rather than strictly before or
  after it, with the landing rule: if S6's own probes demand a check-stage
  grounding change in `terms.rs`, or the two slices' changes interact at merge
  time, S10's checker change lands first and S6 rebases onto it. Growth
  signals (R6) re-run on every touched file at phase exit. Final full gate ×2.

## Goldens

All new goldens in `tests/phase7b_slice10.rs`. Complete fixture text lives in
[slice10-paper-tests](./slice10-paper-tests.md); the error goldens pin
byte-exact text once the implementing phase measures the rendered output
(measure-then-pin).

| Golden | Test name | Behaviour | Fixture |
| --- | --- | --- | --- |
| GA | `third_module_bare_caller_with_ambiguous_headers_is_a_located_error` | before: builds, prints `2`, exit 0 (both import orders — this is S9's own `third_module_bare_caller_dispatches_the_single_shared_env_instantiation`, retired by REQ-6) → after: located ambiguity error naming `Widget`, modules `a`/`b` (lexicographic), `c`'s call site; exit 1; **identical text for both `p1-a-b` and `p1-b-a`** (import-order independence, R4) | `p1-a-b`, `p1-b-a` |
| GB | `third_module_bare_caller_error_is_independent_of_the_eager_minter` | before: prints `1`, exit 0 (`a` is the sole eager minter — the other half of S9's G4) → after: the **same** error text as GA (modules named `a`/`b`, lexicographic — independent of which module minted), exit 1 | `p8-a-eager` |
| GC | `both_modules_eager_2_candidate_ambiguity_error_unchanged` | `error: no overload of \`Widget\` in \`try\` (line 3) accepts these operands` + two `candidate: \`i64\`` lines, exit 1 — **byte-identical** | `p2-both-eager` |
| GD | `single_declaring_header_bare_caller_still_resolves` | prints `7`, exit 0 — **unchanged** (one header program-wide, no export-gate issue — `lib.sth`'s `usesize` is private) | `p3a-single-lib-private` |
| GE | `single_reachable_header_with_selective_import_still_resolves` | prints `1`, exit 0 — **unchanged**. Only module `a` exists in this fixture at all — exemption 2 (≤1 reachable header) fires; the selective import is present but inert, not the reason it stays legal (renamed from the original `selective_type_import_bare_ctor_pins_exporters_impl`, which overclaimed a pinning mechanism this fixture never exercises — no second header exists to pin against) | `p5g2-selective-type` |
| GF | `single_reachable_header_with_qualified_signature_still_resolves` | prints `1`, exit 0 — **unchanged**, same reasoning as GE (no second header exists; the qualified signature is inert; renamed from `qualified_type_spelling_bare_ctor_pins_exporters_impl`) | `p7c3-qualified-type` |
| GG | `unimported_foreign_type_annotation_is_still_an_error` | `error: unknown type \`Widget\` at line 3, col 9`, exit 1 — **unchanged** (type-position rule; S10 governs term-position only) | `p4-c-annotates` |
| GH | `unimported_declaring_module_does_not_count_toward_ambiguity` | prints `7`, exit 0 — **unchanged, both before and after**. Two headers exist program-wide (`lib`, `z`); `app` imports only `lib`; only one header is reachable from `app`, so exemption 2 fires even though a second header exists elsewhere in the closure — the fixture that justifies reachability-scoping the count (R1) rather than a program-wide one | `overfire` |
| GI | `matching_selective_import_of_the_sole_minter_still_resolves` | prints `1`, exit 0 — **unchanged, both before and after**. Two reachable headers (`a`, `b`); only `a` mints; `c` selectively imports `a`'s `Widget`, matching the sole candidate's owning module — exemption 4 fires | `sel1` |
| GJ | `selective_import_pins_the_named_exporter_when_both_mint` | prints `1`, exit 0 — **unchanged, both before and after**. Both `a` and `b` mint (multi-candidate arm); existing S5 tier-2 pinning selects `a` — exemption 3, untouched by S10 | `p5i2-selective-one-of-two` |
| GK | `mismatched_selective_import_is_still_a_located_error` | before: silently prints `2`, exit 0 (`c` selectively imports `a`'s `Widget`, but `b` is the sole eager minter — the caller's own selection is silently overridden) → after: the **same** located error as GA (naming modules `a`/`b`), exit 1 — exemption 4 does not apply since the selected module does not match the sole actual candidate's owning module. Built during the pre-implementation review round specifically to probe exemption 4's soundness; without the match requirement this case would stay silently wrong | `p9-mismatched-selective` (new fixture, see paper-tests) |

## Units (beside the changed code)

- `ambiguous_foreign_headers_grounding_is_located_error` — headerless caller,
  ≥2 same-named headers from reachable distinct modules, single env candidate
  ⇒ the grounding path errors (never falls through to the borrowed mint).
- `single_foreign_header_grounding_still_borrows` — same shape but at most one
  same-named header *reachable* ⇒ current borrow behaviour, covering both "no
  second header exists anywhere" (GD) and "a second header exists but is not
  reachable" (GH's mechanism at unit level).
- `own_header_still_grounded_first` — caller declares its own header ⇒ R1.1a
  grounding, no ambiguity (the caller-owns exemption).
- `ambiguous_header_error_names_declaring_modules` — the rendered message
  contains the surface name, both declaring modules (lexicographically
  ordered), and the call site.
- `reachable_header_count_excludes_unimported_declaring_modules` — a header
  declared by a module absent from the caller's own `imports`/`selective` maps
  does not count toward the ≥2 threshold (GH's mechanism at unit level).
- `matching_selective_import_exempts_the_ambiguity_check` — the caller's
  `selective` map names exactly the sole candidate's owning module ⇒
  exemption 4 fires, no error (GI's mechanism at unit level).
- `mismatched_selective_import_does_not_exempt_the_ambiguity_check` — the
  caller's `selective` map names a *different* reachable module than the sole
  candidate's owning module ⇒ exemption 4 does **not** fire, the error still
  raises (GK's mechanism at unit level — the soundness-critical case).

## Guardrails (carried from S9, verbatim in intent)

- **R-NFR1** — no IR/lowering edits. S10 is **check-stage only**; S9's sole
  sanctioned `ir/layout.rs` exception (its duplicated-name layout keys) does
  **not** carry into S10. Any candidate fix needing an IR/lowering edit stops and
  escalates.
- **R-NFR2** — `match_impl_target`/`..._rec` and `find_bound_impl`'s scan have
  zero behavioural diff; the dispatched `size` member call routes exactly as
  today whenever grounding succeeds.
- **R-NFR3** — no run-count / ratio assertions on the new goldens; determinism is
  pinned by identical byte-exact text across import orders and minter placements
  (REQ-5), not by a cycle ratio.
- Diagnostics are behaviour: only the NEW message (R3/R5) is new. No existing
  diagnostic text churns (GC pins the 2-candidate text unchanged).

## Out of scope (pre-existing warts, recorded not fixed)

`export: Widget[i64]` parse error and the R18 gate's unsatisfiable instantiation
remedy (P5a/P5f); qualified ctor **term** `a::Widget` → unknown word (P7b2); the
concrete-type same-name collision dimension; a distinguishable naming phrasing
for 2+ declaring modules reached *only* through wildcard imports (R4; no
fixture in this spec exercises it). None may be assumed as a workaround; none
are fixed by S10 (the policy may not assume "export the word over the type").

## Open questions (resolved before /implement)

- **OQ-1 (error wording).** New located message vs extending
  `no_overload_matches_error` for both shapes. **Resolved: new message; no
  churn to the existing 2-candidate text** (R3; GC pins it).
- **OQ-2 (sequencing).** Implement S10 before or after the pending ladder
  slices S6–S8? **Resolved by maintainer ruling (interview, 2026-09-05): S10
  is implemented in parallel with P7b.S6**, both branching from base `a9eca84`
  — not strictly before or after. Landing rule: if S6's own probes demand a
  check-stage grounding change in `terms.rs`, or the two slices' changes
  interact at merge time, S10's checker change lands first and S6 rebases onto
  it. The roadmap records "S10 ∥ S6", not a strict ladder position (REQ-8).
- **OQ-3 (remedy note).** Should the error's remedy note mention the future
  qualified-ctor-term syntax (P7b2), or qualified type spelling generally?
  **Resolved: neither.** Qualified spelling is dropped from the note entirely,
  not phrased as "not yet available" — measurement during the
  pre-implementation review round showed it never cures the single-candidate
  shape (it just mints a second candidate, routing into the pre-existing
  2-candidate error). See R5.

## Baseline (measured at cd44b1c / a9eca84)

- Suite: **3175 passing / 0 failed** at `cd44b1c` (the slice4 flake is gone
  since S9 Phase 3; expect zero failures); `a9eca84` adds only this slice's own
  docs, no code, so the baseline is unaffected. Re-measure with
  `--no-fail-fast`.
- Test binaries: **80 integration files** (`tests/*.rs`) + lib unit-tests + bin
  unit-tests = **82 test binaries**. S10 adds `tests/phase7b_slice10.rs` → **83**.
  (Slice9-spec's JSON baseline counted 82 at the earlier base `600bc1b`; numbers
  drift with the base, so the implementing phase re-measures from `a9eca84`.)
- Green = `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
  (baseline `clippy --all-targets` separately; it is red at HEAD independent of
  this slice — stash-baseline before trusting it).

## Delivery plan

### Phase 1 — policy + goldens (difficulty: standard)

**Changes.**

- Implement R1/R2/R3/R4/R5 at the single-candidate grounding fall-through in
  `src/check/terms.rs` (`bare_generated_word_own_module_grounding`,
  `1507-1509`): when the caller has no own header, the one env candidate is
  foreign, and `ctx.generics().structs` holds ≥2 same-named headers from ≥2
  distinct modules *reachable through the caller's own `ModuleInfo.imports`/
  `.selective`* (none the caller's), emit the new located ambiguity error
  unless exemption 4 (a matching explicit resolution) applies. No edit to
  `select_overload`, `tier_pick`, env build, or the matcher.
- Retire or rewrite S9's `third_module_bare_caller_dispatches_the_single_shared_env_instantiation`
  golden (`tests/phase7b_slice9.rs`) — REQ-6 is explicit that this one S9
  golden inverts (it currently pins the exact silent outputs GA/GB replace
  with an error); every other S9 golden stays green and byte-unchanged.

**Goldens.** GA, GB, GK (new located error, byte-exact, deterministic across
import orders + minter placement + the mismatched-selective-import case) +
regression pins GC, GD, GE, GF, GG, GH, GJ — all in `tests/phase7b_slice10.rs`.

**Units.** `ambiguous_foreign_headers_grounding_is_located_error`,
`single_foreign_header_grounding_still_borrows`, `own_header_still_grounded_first`,
`ambiguous_header_error_names_declaring_modules`,
`reachable_header_count_excludes_unimported_declaring_modules`,
`matching_selective_import_exempts_the_ambiguity_check`,
`mismatched_selective_import_does_not_exempt_the_ambiguity_check` (beside the
changed `terms.rs` code).

**Exit.** GA/GB/GK error byte-exact and deterministic (both import orders,
both minter placements, the mismatched-selective-import case); GC/GD/GE/GF/GG/GH/GJ
byte-identical to today; S9's G4 golden retired/rewritten, every other S9
golden + #10 + S5 tier-1 green; new error text pinned; full gate green
(3175+N / 0, less S9's one retired test if replaced rather than rewritten).
Growth signals noted on `terms.rs` (deferred to Phase 2's formal re-check).

**Notes.** Measure-then-pin: pin whatever the formatter renders for the new
message; the R5 draft is the contract, the golden is the bytes. R-NFR1: no IR
edit — if the check needs one, stop and escalate. Exemption 4's match test
(R1) is the soundness-critical piece of this phase — do not implement it as a
bare "selective import present" check; GK's golden exists specifically to
catch that regression.

### Phase 2 — roadmap + growth re-check + final gate (difficulty: standard)

**Changes.** Update the roadmap S9 entry's Exit clause
(`docs/roadmap/P7b-higher-kinded-types.md`) — its trailing "Residual: ...
future work" sentence — to state the shape is **closed by P7b.S10**; add a new
**P7b.S10** entry in current-design prose (no history narration, per
feedback), pointing at this spec and recording the S10 ∥ S6 parallel
sequencing and landing rule (REQ-8). No code change.

**Goldens.** None new (documentation phase).

**Units.** None.

**Exit.** Roadmap S9 Exit clause no longer describes an open hole; S10 entry
present, recording the parallel-with-S6 sequencing; growth signals (R6)
re-run on every file S10 touched (`src/check/terms.rs`,
`tests/phase7b_slice10.rs`, `tests/phase7b_slice9.rs`, roadmap docs) with the
outcome recorded; final full gate ×2 green.

**Notes.** ROADMAP/DESIGN carry current design only, never history narration.

## Phases (JSON)

```json
{
  "phases": [
    {
      "id": 1,
      "name": "policy + goldens",
      "difficulty": "standard",
      "requirements": ["REQ-1", "REQ-2", "REQ-3", "REQ-4", "REQ-5", "REQ-6", "REQ-7"],
      "changes": [
        "Implement R1/R2/R3/R4/R5 at the single-candidate grounding fall-through in src/check/terms.rs (bare_generated_word_own_module_grounding, 1507-1509 region): headerless caller + one foreign env candidate + >=2 same-named headers from >=2 distinct modules reachable through the caller's own ModuleInfo.imports/.selective (none the caller's) => new located ambiguity error, unless exemption 4 (a matching explicit resolution via ModuleInfo.selective) applies",
        "Read header provenance from ctx.generics().structs; read the caller's import closure from ctx.modules (ModuleInfo.imports/.selective); no edit to select_overload, tier_pick, env build (check.rs:586), or the matcher (find_bound_impl / match_impl_target)",
        "Retire or rewrite S9's third_module_bare_caller_dispatches_the_single_shared_env_instantiation golden (tests/phase7b_slice9.rs), which currently pins the exact silent outputs GA/GB replace with an error"
      ],
      "goldens": [
        "third_module_bare_caller_with_ambiguous_headers_is_a_located_error",
        "third_module_bare_caller_error_is_independent_of_the_eager_minter",
        "both_modules_eager_2_candidate_ambiguity_error_unchanged",
        "single_declaring_header_bare_caller_still_resolves",
        "single_reachable_header_with_selective_import_still_resolves",
        "single_reachable_header_with_qualified_signature_still_resolves",
        "unimported_foreign_type_annotation_is_still_an_error",
        "unimported_declaring_module_does_not_count_toward_ambiguity",
        "matching_selective_import_of_the_sole_minter_still_resolves",
        "selective_import_pins_the_named_exporter_when_both_mint",
        "mismatched_selective_import_is_still_a_located_error"
      ],
      "units": [
        "ambiguous_foreign_headers_grounding_is_located_error",
        "single_foreign_header_grounding_still_borrows",
        "own_header_still_grounded_first",
        "ambiguous_header_error_names_declaring_modules",
        "reachable_header_count_excludes_unimported_declaring_modules",
        "matching_selective_import_exempts_the_ambiguity_check",
        "mismatched_selective_import_does_not_exempt_the_ambiguity_check"
      ],
      "exit": "GA/GB/GK error byte-exact and deterministic across both import orders, both minter placements, and the mismatched-selective-import case; GC/GD/GE/GF/GG/GH/GJ byte-identical to today; S9's G4 golden retired/rewritten, every other S9 golden (G1/G1a-f/G2/G2r/G3) + #10 + S5 tier-1 green; new error text pinned (measure-then-pin); full gate green."
    },
    {
      "id": 2,
      "name": "roadmap correction + growth re-check + final gate",
      "difficulty": "standard",
      "requirements": ["REQ-8"],
      "changes": [
        "Update the roadmap S9 entry's Exit clause (its trailing Residual sentence) in docs/roadmap/P7b-higher-kinded-types.md to state the shape is closed by P7b.S10",
        "Add a new P7b.S10 roadmap entry in current-design prose (no history narration), pointing at this spec and recording the S10-parallel-with-S6 sequencing and landing rule"
      ],
      "goldens": [],
      "units": [],
      "exit": "Roadmap S9 Exit clause no longer describes an open hole; S10 entry present recording the parallel sequencing; growth signals (R6) re-run on every file S10 touched with outcome recorded; final full gate x2 green."
    }
  ]
}
```

## Requirement → phase map

| REQ | Phase | Goldens / units |
| --- | --- | --- |
| REQ-1 policy | 1 | GA, GB, GD, GH, GC, GI, GK; ambiguous/single/own/reachable/matching/mismatched units |
| REQ-2 layer | 1 | own_header_still_grounded_first |
| REQ-3 diagnostic | 1 | GA, GB, GC, GK; ambiguous_header_error_names_declaring_modules |
| REQ-4 compat pins | 1 | GC, GD, GE, GF, GG, GH, GJ |
| REQ-5 determinism + naming | 1 | GA, GB, GK |
| REQ-6 S9 goldens (one inverts) | 1 | S9's G4 retired/rewritten; G1/G1a-f, G2, G2r, G3, #10, S5 tier-1 untouched |
| REQ-7 guardrails | 1 | (R-NFR1/2/3 across all Phase-1 goldens) |
| REQ-8 roadmap + growth + gate | 2 | (docs; final gate x2) |
