# P7b.S10 spec — header-level export ambiguity for the third-module bare caller

> Delivery spec. Companion frozen docs: [slice10-brief](./slice10-brief.md)
> (rulings R1/R2/R3, constraints, open questions), [slice10-paper-tests](./slice10-paper-tests.md)
> (complete fixtures + measured-before columns + unit sketches),
> [slice10-probes](./slice10-probes.md) (verbatim probe log + verdict). Read
> [slice9-spec](./slice9-spec.md) for the two-defect story and the
> R1.1a / R2.4 / R-NFR context this slice sits beside. Base: `cd44b1c`
> (P7b.S9 merged to `main`; suite 3175/0).

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
header registry — not on the trait-impl matcher (R4/R-NFR2 forbid the
dispatch-time machinery a naive fix would reach for).

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
  (`src/check/terms.rs:1516-1523` region): caller has no own header
  (`find_struct(name, caller_module)` → `None`), one env candidate exists, it is
  used unchanged.
- The S5 tier policy cannot see the latent ambiguity: with one env candidate
  `select_overload`/`tier_pick` never error (lone-survivor ruling,
  `src/check/builtins.rs:118-127`); with two env candidates the existing
  accepted-ambiguity error already fires (P2), naming no modules.

## Rulings

### R1 — the policy (header-level accepted ambiguity)

When a bare ctor/destructure call in module `m` reaches the single-candidate arm
with exactly **one** env candidate whose mint's header belongs to a foreign
module, and the generic header registry holds **≥2 same-named headers declared
by ≥2 distinct modules, none of them `m`**, the grounding raises a **located
compile-time ambiguity error** at the call site naming the surface name, the ≥2
declaring modules, and the call span, with remedy pointers. The silent pick
becomes an error; the minter stops deciding (GA/GB both error, regardless of
which module spells eagerly).

**Exemptions — the error must NOT fire when:**

- `m` declares its own header — S9's R1.1a grounds the caller's own mint (the
  caller-owns tier, untouched);
- exactly **one** same-named header exists program-wide — the single-lib compat
  shape (GD) stays legal and silent-by-design;
- the call reaches the **multi-candidate** arm (≥2 env candidates) — the
  existing S5 `select_overload` path governs there, including tier-2 pinning for
  declared type imports (GE/GF, P5i2), byte-identical.

### R2 — where the check lives

At the single-candidate arm's grounding check in `src/check/terms.rs`
(`1516-1523` region) — reading caller span, the foreign candidate, and header
provenance from the generic registry (`ctx.generics()` / `module.generic_structs`,
complete at env-build time per P6). **NOT** at `select_overload`/`tier_pick`
(cannot see 1-candidate shapes; the lone-survivor ruling deliberately never
errors on one candidate). **NOT** at env build (`src/check.rs:586`; no call site
to locate an error at). **NOT** in the matcher (`find_bound_impl` /
`match_impl_target` — S9's R-NFR2 carried over: the dispatched `size` member call
keeps routing through `resolve_mono_member_call` → `find_bound_impl` exactly as
today whenever grounding succeeds).

### R3 — diagnostics

A **NEW** located message in house style (`error: \`Widget\` in \`try\` (line N,
col M) is ambiguous: ...` + a `note:` remedy line). It names the surface name,
the ≥2 declaring modules, and the call site, and points at the three remedies
that exist today. The existing 2-candidate `no_overload_matches_error` text is
**not churned** (diagnostics are behaviour; GC pins it byte-unchanged).

### R4 — the exact wording (contract; the golden pins the measured bytes)

The new message follows the measure-then-pin discipline: this spec fixes the
wording **contract** below; the implementing phase measures the real rendered
output and pins whatever the formatter actually emits (spacing, the parenthetical
`(line N, col M)` shape, and how module names are joined all follow the existing
`terms.rs` diagnostic helpers). Draft contract:

```
error: `Widget` in `try` (line 3, col 22) is ambiguous: declared in modules `a` and `b`, and `try`'s module declares no `Widget`
  note: declare your own `Widget` header, or selectively import one type (`import: self::a | Widget | ;` after `export: Widget ;`), or spell the type qualified (`a::Widget[i64]`)
```

Contract, byte-exact wording deferred to the golden:

- lead line: surface name in backticks, the enclosing word in backticks, the
  `(line N, col M)` call site, the word "ambiguous", and the ≥2 declaring module
  names (order deterministic — see R5);
- note line: the **three** remedies that work today — (1) declare your own
  `Widget` header; (2) selective type import (`import: self::a | Widget | ;`
  after `export: Widget ;`); (3) qualified type spelling (`a::Widget[i64]`).
  The future qualified-ctor-term syntax (P7b2) is **not** mentioned (OQ-3
  default).

## Requirements

- **REQ-1 (policy, R1).** At the single-candidate grounding arm, a bare
  ctor/destructure call in module `m` with exactly one foreign env candidate
  fires the located ambiguity error **iff** the generic registry holds ≥2
  same-named headers from ≥2 distinct modules, none of them `m`. The three
  exemptions (own header; one header program-wide; multi-candidate arm) hold
  exactly. Traces to GA, GB, GD, GC; units `ambiguous_foreign_headers_grounding_is_located_error`,
  `single_foreign_header_grounding_still_borrows`, `own_header_still_grounded_first`.
- **REQ-2 (layer, R2).** The check lives at the `terms.rs:1516-1523` grounding
  fall-through, reading header provenance from `ctx.generics()` /
  `module.generic_structs`. No edit to `select_overload`, `tier_pick`, env build
  (`check.rs:586`), or the matcher (`find_bound_impl`/`match_impl_target`).
  Traces to REQ-7 guardrails; unit `own_header_still_grounded_first`.
- **REQ-3 (diagnostic, R3/R4).** A new located message naming surface name, ≥2
  declaring modules, and call site, with the three-remedy note; the existing
  2-candidate `no_overload_matches_error` text is byte-unchanged. Traces to GA,
  GB, GC; unit `ambiguous_header_error_names_declaring_modules`.
- **REQ-4 (compat pins).** GD/GE/GF/GG byte-identical to today; GC byte-identical;
  P5i2 tier-2 selective-import pinning untouched (not re-errored). Traces to
  GC/GD/GE/GF/GG.
- **REQ-5 (determinism).** GA/GB error **identically** (byte-exact) regardless of
  which module is the eager minter and regardless of import order — the declaring
  modules named in the message are ordered deterministically (registry order,
  not env/mint order). No run-count ratio asserted (R-NFR3). Traces to GA, GB.
- **REQ-6 (S9 goldens untouched).** S9's G1/G1a–G1f, G2, G2r, G3, G4, plus #10
  and S5 tier-1 stay green and byte-unchanged — the new check sits beside R1.1a's
  grounding and only fires where R1.1a falls through today.
- **REQ-7 (guardrails).** R-NFR1 (no IR/lowering edits — and the S9 `ir/layout.rs`
  exception does **not** carry into S10; S10 is check-stage only), R-NFR2 (matcher
  untouched), R-NFR3 (no ratio assertions on the new goldens). Diagnostics are
  behaviour: only the NEW message is new; no existing diagnostic text churns.
- **REQ-8 (roadmap + growth + gate).** The roadmap S9 entry's Residual sentence
  updated to "closed by P7b.S10"; a new S10 entry in current-design prose (no
  history narration). Growth signals (R6) re-run on every touched file at phase
  exit. Final full gate ×2.

## Goldens

All new goldens in `tests/phase7b_slice10.rs`. Complete fixture text lives in
[slice10-paper-tests](./slice10-paper-tests.md); the error goldens pin byte-exact
text once the implementing phase measures the rendered output (measure-then-pin).

| Golden | Test name | Behaviour | Fixture |
| --- | --- | --- | --- |
| GA | `third_module_bare_caller_with_ambiguous_headers_is_a_located_error` | before: builds, prints `2`, exit 0 → after: located ambiguity error naming `Widget`, modules `a`/`b`, `c`'s call site; exit 1 | `p1-a-b` |
| GB | `third_module_bare_caller_error_is_independent_of_the_eager_minter` | before: prints `1`, exit 0 (only `a` eager) → after: the **same** error as GA, exit 1 | minter-swap twin of `p1-a-b` |
| GC | `both_modules_eager_2_candidate_ambiguity_error_unchanged` | `error: no overload of \`Widget\` in \`try\` (line 3) accepts these operands` + two `candidate: \`i64\`` lines, exit 1 — **byte-identical** | `p2-both-eager` |
| GD | `single_declaring_header_bare_caller_still_resolves` | prints `7`, exit 0 — **unchanged** (one header program-wide) | `p3-single-lib` |
| GE | `selective_type_import_bare_ctor_pins_exporters_impl` | prints `1`, exit 0 — **unchanged** (tier-2 pinning, P5i2) | `p5g2-selective-type` |
| GF | `qualified_type_spelling_bare_ctor_pins_exporters_impl` | prints `1`, exit 0 — **unchanged** (qualified spelling) | `p7c3-qualified-type` |
| GG | `unimported_foreign_type_annotation_is_still_an_error` | `error: unknown type \`Widget\` at line 3, col 9`, exit 1 — **unchanged** (type-position rule; S10 governs term-position only) | `p4-c-annotates` |

## Units (beside the changed code)

- `ambiguous_foreign_headers_grounding_is_located_error` — headerless caller, ≥2
  same-named headers from distinct modules in the generic registry, single env
  candidate ⇒ the grounding path errors (never falls through to the borrowed
  mint).
- `single_foreign_header_grounding_still_borrows` — same shape but exactly one
  same-named header ⇒ current borrow behaviour (GD's mechanism at unit level).
- `own_header_still_grounded_first` — caller declares its own header ⇒ R1.1a
  grounding, no ambiguity (the caller-owns exemption).
- `ambiguous_header_error_names_declaring_modules` — the rendered message
  contains the surface name, both declaring modules, and the call site.

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
- Diagnostics are behaviour: only the NEW message (R3/R4) is new. No existing
  diagnostic text churns (GC pins the 2-candidate text unchanged).

## Out of scope (pre-existing warts, recorded not fixed)

`export: Widget[i64]` parse error and the R18 gate's unsatisfiable instantiation
remedy (P5a/P5f); qualified ctor **term** `a::Widget` → unknown word (P7b2); the
concrete-type same-name collision dimension. None may be assumed as a workaround;
none are fixed by S10 (the policy may not assume "export the word over the type").

## Open questions (settled by default — flagged for user override before /implement)

- **OQ-1 (error wording).** New located message vs extending
  `no_overload_matches_error` for both shapes. **Default (taken): new message; no
  churn to the existing 2-candidate text** (R3; GC pins it).
- **OQ-2 (sequencing).** Implement S10 before or after the pending ladder slices
  S6–S8. **Default (taken): before** — small, independent, closes a correctness
  gap in the same area; S6–S8 build on the trait ladder, not on this policy.
- **OQ-3 (remedy note).** Mention the future qualified-ctor-term syntax (P7b2) as
  "not yet available". **Default (taken): no** — the note lists only the three
  remedies that work today.

## Baseline (measured at cd44b1c)

- Suite: **3175 passing / 0 failed** (the slice4 flake is gone since S9 Phase 3;
  expect zero failures). Re-measure with `--no-fail-fast`.
- Test binaries: **80 integration files** (`tests/*.rs`) + lib unit-tests + bin
  unit-tests = **82 test binaries**. S10 adds `tests/phase7b_slice10.rs` → **83**.
  (Slice9-spec's JSON baseline counted 82 at the earlier base `600bc1b`; numbers
  drift with the base, so the implementing phase re-measures from cd44b1c.)
- Green = `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
  (baseline `clippy --all-targets` separately; it is red at HEAD independent of
  this slice — stash-baseline before trusting it).

## Delivery plan

### Phase 1 — policy + goldens (difficulty: standard)

**Changes.** Implement R1/R2/R3 at the single-candidate grounding fall-through in
`src/check/terms.rs` (the `1516-1523` region): when the caller has no own header,
the one env candidate is foreign, and `ctx.generics()` /
`module.generic_structs` holds ≥2 same-named headers from ≥2 distinct modules
(none the caller's), emit the new located ambiguity error (R4 contract) instead
of borrowing the mint. No edit to `select_overload`, `tier_pick`, env build, or
the matcher.

**Goldens.** GA, GB (new located error, byte-exact, deterministic across import
orders + minter placement) + regression pins GC, GD, GE, GF, GG — all in
`tests/phase7b_slice10.rs`.

**Units.** `ambiguous_foreign_headers_grounding_is_located_error`,
`single_foreign_header_grounding_still_borrows`, `own_header_still_grounded_first`,
`ambiguous_header_error_names_declaring_modules` (beside the changed `terms.rs`
code).

**Exit.** GA/GB error byte-exact and deterministic (both import orders, both
minter placements); GC/GD/GE/GF/GG byte-identical to today; S9 goldens + #10 + S5
tier-1 green; new error text pinned; full gate green (3175+N / 0). Growth signals
noted on `terms.rs` (deferred to Phase 2's formal re-check).

**Notes.** Measure-then-pin: pin whatever the formatter renders for the new
message; the R4 draft is the contract, the golden is the bytes. R-NFR1: no IR
edit — if the check needs one, stop and escalate.

### Phase 2 — roadmap + growth re-check + final gate (difficulty: standard)

**Changes.** Update the roadmap S9 entry's Residual sentence
(`docs/roadmap/P7b-higher-kinded-types.md`) to state the shape is **closed by
P7b.S10**; add a new **P7b.S10** entry in current-design prose (no history
narration, per feedback), pointing at this spec. No code change.

**Goldens.** None new (documentation phase).

**Units.** None.

**Exit.** Roadmap S9 Residual no longer describes an open hole; S10 entry present;
growth signals (R6) re-run on every file S10 touched (`src/check/terms.rs`,
`tests/phase7b_slice10.rs`, roadmap docs) with the outcome recorded; final full
gate ×2 green.

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
        "Implement R1/R2/R3 at the single-candidate grounding fall-through in src/check/terms.rs (1516-1523 region): headerless caller + one foreign env candidate + >=2 same-named headers from >=2 distinct modules (none the caller's) => new located ambiguity error instead of borrowing the mint",
        "Read header provenance from ctx.generics() / module.generic_structs; no edit to select_overload, tier_pick, env build (check.rs:586), or the matcher (find_bound_impl / match_impl_target)"
      ],
      "goldens": [
        "third_module_bare_caller_with_ambiguous_headers_is_a_located_error",
        "third_module_bare_caller_error_is_independent_of_the_eager_minter",
        "both_modules_eager_2_candidate_ambiguity_error_unchanged",
        "single_declaring_header_bare_caller_still_resolves",
        "selective_type_import_bare_ctor_pins_exporters_impl",
        "qualified_type_spelling_bare_ctor_pins_exporters_impl",
        "unimported_foreign_type_annotation_is_still_an_error"
      ],
      "units": [
        "ambiguous_foreign_headers_grounding_is_located_error",
        "single_foreign_header_grounding_still_borrows",
        "own_header_still_grounded_first",
        "ambiguous_header_error_names_declaring_modules"
      ],
      "exit": "GA/GB error byte-exact and deterministic across both import orders and both minter placements; GC/GD/GE/GF/GG byte-identical to today; S9 goldens (G1/G1a-f/G2/G2r/G3/G4) + #10 + S5 tier-1 green; new error text pinned (measure-then-pin); full gate green (3175+N/0)."
    },
    {
      "id": 2,
      "name": "roadmap correction + growth re-check + final gate",
      "difficulty": "standard",
      "requirements": ["REQ-8"],
      "changes": [
        "Update the roadmap S9 entry Residual sentence in docs/roadmap/P7b-higher-kinded-types.md to state the shape is closed by P7b.S10",
        "Add a new P7b.S10 roadmap entry in current-design prose (no history narration), pointing at this spec"
      ],
      "goldens": [],
      "units": [],
      "exit": "Roadmap S9 Residual no longer describes an open hole; S10 entry present; growth signals (R6) re-run on every file S10 touched with outcome recorded; final full gate x2 green."
    }
  ]
}
```

## Requirement → phase map

| REQ | Phase | Goldens / units |
| --- | --- | --- |
| REQ-1 policy | 1 | GA, GB, GD, GC; ambiguous/single/own units |
| REQ-2 layer | 1 | own_header_still_grounded_first |
| REQ-3 diagnostic | 1 | GA, GB, GC; ambiguous_header_error_names_declaring_modules |
| REQ-4 compat pins | 1 | GC, GD, GE, GF, GG |
| REQ-5 determinism | 1 | GA, GB |
| REQ-6 S9 goldens | 1 | G1/G1a-f, G2, G2r, G3, G4, #10, S5 tier-1 |
| REQ-7 guardrails | 1 | (R-NFR1/2/3 across all Phase-1 goldens) |
| REQ-8 roadmap + growth + gate | 2 | (docs; final gate x2) |
