# P7b.S11 — per-call-site grounding for bare generic constructors

> Companion frozen docs: [slice11-brief](./slice11-brief.md) (problem, mechanism,
> rulings R1–R7, proposed goldens), [slice11-probes](./slice11-probes.md) (verbatim
> dp_a–dp_h round with Q1–Q3 analysis), byte-exact stderr baseline
> `probes/dp_baseline.md`, fixtures `probes/dp_*.sth`. Sits beside
> [slice10-spec](./slice10-spec.md) (the cross-module header-ambiguity sibling; this
> slice is the same-module counterpart and is verified structurally separate from it).
> Base `7404a71` (main; the dp round ran at this HEAD, suite green). Re-verified
> at `486eda4` (merge p7b-s8c; main has since landed S8, S8b, S8c): the frozen
> baseline `probes/dp_baseline.md` holds byte-for-byte at `486eda4` (all four
> stderr shapes identical, all 11 exit codes match `probes/dp_findings.md`,
> `cargo test --test phase7b_slice10` 17 passed / 0 failed), and every anchor
> below is a fresh `486eda4` line. `parser.rs`, `ast.rs`, and `check.rs` are
> byte-identical to `7404a71` — no parse-side or env-build drift exists — and
> the only `src/ir` diff in `7404a71..486eda4` is a doc-comment rewrite.
> Invocation `cargo run -q -- run probes/<name>.sth --manifest
> tests/fixtures/sooth.pkg`.

## Why

Bare generic constructor/destructure calls (`Ok`, `Err`, and enum/struct
generated words with a free type parameter) have **no per-call-site grounding**.
The env is built once from parse-time eager mints only (`check.rs:579-603`, the
`struct_generated_sigs` population loop at `:586`); a bare ctor call misses
`env` and falls to `mint_fallback_candidates` (`terms.rs:2024`), which scans
the whole-module monomorph registry for a name match with no call-site
information. This produces four measured defects (probe round, verbatim in
slice11-probes):

1. **Zero-mint misdiagnostic (dp_c).** `1 Ok drop` with no concrete `Res[...]`
   application anywhere in the module → the generic `unknown word` error
   (`terms.rs:923`), indistinguishable from a genuinely undefined name and blaming
   the wrong word rather than the unbound parameter `'E`.
2. **Wrong-mint forcing (dp_d).** A sole existing mint is taken unconditionally
   (the chosen `[only]` arm, `terms.rs:950-991`) regardless of fit. `1 mkok Ok
   drop` forces the inner `Ok` onto the unrelated `Res[i64 i64]` mint from
   `mkok`'s signature
   and fails later with a far-away operand mismatch that never mentions grounding.
   Rejection is correct; the mechanism reaching it is accidental.
3. **Silent first-wins on ties (dp_g2/dp_g3).** With 2+ mints,
   `select_overload_fallback_sourced` (`builtins.rs:164-183`) falls back to
   `matching.first()` — declaration order — with no ambiguity check. Two programs
   differing only in unrelated declaration order construct different runtime types
   from the same bare call, silently. **A correctness gap, not a diagnostics gap.**
4. **No explicit-args category (dp_e/dp_e2).** `Ok[i64 i64]` is rejected at
   `poly_call_takes_type_args` (`terms.rs:1314-1348`): only user-declared poly
   words and trait members admit type args. No escape-hatch spelling exists,
   independent of grounding; the consumer is irrelevant (dp_e2 byte-identical).

**Round-2 status at `486eda4`: S8b/S8c pre-empt none of this slice.** S8c's
per-site binding (`compose_member_theta`, `poly.rs:9415`; site-slot
compatibility check, `poly.rs:9506`) hooks bound dispatch only — none of S11's
four capabilities (per-site θ for generated words, consumer-driven constraints
landing at the resolve loop, compatibility filtering of mint candidates, the
explicit-args category) is built on the ctor path, and
`mint_fallback_candidates` is untouched by both landings. The one newcomer
inside the restructuring target is S8b's span-keyed pin (`terms.rs:974-988`),
which the plan below must preserve.

This is the silent load-bearing dependence the roadmap exists to turn into sharp
compile errors (CLAUDE.md). Landing ahead of P9.S2 keeps the alloc-layer brief
free of checker-grounding concerns; this slice is **checker-stage only** and has
no forcing dependency on P9.

## Requirements

Each requirement is independently verifiable against a golden or unit.

- **R-1 (obligation-style re-grounding).** A bare generic ctor/destructure call
  whose name has a known generic header but is unresolved at the `env.get`-miss
  branch is re-grounded at the resolve loop from call-site inputs, not from the
  incidental whole-module mint scan. Grounding derives a substitution θ for the
  header's type parameters; the monomorph selected is the one θ names, whether or
  not it was already minted.

- **R-2 (grounding inputs, precedence order — R3 of brief).** Inputs are consumed
  in this order:
  1. **Explicit type args** (full arity, per R-6) — pin all parameters directly.
  2. **Consumer-driven re-grounding** (S2-9 obligation style) — the consumer's
     constraint lands at the resolve loop and pins parameters it determines.
  3. **Literal-driven partial inference** — operand types at the call site
     (`1` → `i64`) pin the leading parameters they cover.
  Parameters still unbound after all three → located **unbound type parameter**
  error (R-4).

- **R-3 (mints are candidates, not verdicts — R4 of brief).** An existing mint
  participates in compatibility selection like any other candidate; it is never
  taken unconditionally. Selection is compatibility-conditioned against the θ
  derived in R-1/R-2. A sole *incompatible* mint (dp_d) → located grounding error
  naming expected type vs the mint's type, replacing the far-away operand
  mismatch.

- **R-4 (tie-break — R2 of brief).** Precedence: explicit type args > single
  compatible candidate > located ambiguity error. **First-wins is retired** in
  this path. When 2+ candidates are compatible, emit a located ambiguity error
  listing the tied candidate types **sorted by rendered type string**, so the text
  is stable across declaration orders (S10 GA discipline). The zero-mints-with-an-
  unbound-parameter case emits the R-2 unbound-parameter error, not `unknown word`.

- **R-5 (located diagnostics).** Three new located, house-style messages:
  - **unbound type parameter**: names the parameter (`'E`), its position in the
    header, and the remedy (explicit args or a concretely-typed consumer). dp_c.
  - **incompatible sole mint**: names expected type vs the mint's type. dp_d.
  - **ambiguous grounding**: lists tied candidate types sorted by rendered string.
    dp_g2/dp_g3.
  Each is byte-exact (measure-then-pin).

- **R-6 (explicit-args category — R7 of brief, dp_e/dp_e2).**
  `poly_call_takes_type_args` (`terms.rs:1314-1348`) admits a new category: a bare
  generic ctor/destructure name paired with a matching header. Explicit args
  require **full arity** (`Ok[i64 i64]`); prefix pinning (`Ok[i64]` with `'E` from
  context) is out of scope (open question). A wrong-arity explicit-args list is a
  located error. The category is independent of whether a consumer exists (dp_e2:
  the fully-concrete construction then reaches the ordinary forgetting check).

- **R-7 (undefined-name distinguishability — G7).** A genuinely undefined ctor
  name (no known generic header anywhere) still emits the unchanged `unknown word`
  error. Zero-mints-of-a-known-header (R-2 unbound-parameter) must be
  distinguishable from undefined-name.

- **R-8 (monomorph identity/dedup preserved — R6 of brief, P7.S3t).** Re-grounding
  mints or reuses under the same identity discipline: one symbol per `(word, θ)`,
  canonical sorting per P7.S3t. No duplicate or divergent monomorph is introduced.

## Non-functional requirements

- **NFR-1 (checker-stage fence — R6 of brief, re-anchored to behavior).** No
  *behavioral* IR/lowering/emit change relative to `486eda4`; all edits in
  `src/check/`. The byte-level "zero IR diff" fence is retired — the only
  `src/ir` diff in `7404a71..486eda4` is a doc-comment rewrite, so bytes were
  never the operative guarantee. The `foreign_single_candidate_grounding`
  contract (`terms.rs:1690`; entry condition still "candidates already collapsed
  to `[only]`"; untouched by S8b/S8c) is preserved; the `env.get`-miss branch is
  restructured around it, not through it.
- **NFR-2 (behavioral fence — R5 of brief).** No correct program regresses.
  dp_a/dp_b/dp_f behaviors byte-identical to baseline. S10's diagnostics
  byte-unchanged (baseline diff). S5's declared-overload tier policy and S2-9's
  member dispatch untouched.
- **NFR-3 (determinism).** Ambiguity text is order-stable across declaration
  orders (candidate types sorted by rendered string). No run-count/ratio
  assertions; determinism pinned by byte-exact text across orderings.
- **NFR-4 (no cross-module churn).** Cross-module header ambiguity remains S10's
  territory; ties in this slice are same-module mints of one header (verified
  structurally separate at `486eda4`: the fall-through `terms.rs:889-927` is
  upstream of and distinct from `foreign_single_candidate_grounding`
  (`terms.rs:1690`), which is reachable only via
  `bare_generated_word_own_module_grounding` (`terms.rs:1483`) from the
  single-candidate pre-guard).

## Observable success criteria

Goldens (`tests/phase7b_slice11.rs`), measure-then-pin, byte-exact on error text:

| Golden | Shape | Behavior |
| --- | --- | --- |
| G1 | dp_c (`1 Ok drop`) | located unbound-parameter error naming `'E`, byte-exact; not `unknown word` |
| G2 | dp_d (nested, sole wrong mint) | located grounding error naming expected vs mint, byte-exact |
| G3 | dp_g2 + dp_g3 (tied mints, both orders) | located ambiguity error, byte-identical across the two declaration orders |
| G4 | dp_e (`1 Ok[i64 i64] drop`) | accepted, runs clean |
| G5 | `1 Ok [ 1 sub ] map[i64 i64 i64] drop` | accepted via consumer-driven grounding (map's args pin the channels), runs, expected output |
| G6 | dp_a / dp_b / dp_f | behavior byte-identical to baseline (non-regression) |
| G7 | genuinely undefined ctor name | `unknown word` byte-unchanged |
| G8 | S10 shapes (`probes/dp_baseline.md`) | stderr byte-identical; slice10 goldens stay green |

Plus ~12 units beside each changed site (`terms.rs` gate + fall-through,
`builtins.rs` fallback, `parser.rs` if the args category needs parse support),
named `thing_condition_expected`.

Gate: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`, green,
with the S10 golden suite byte-unchanged.

## Scope and boundaries

**In scope:** per-call-site grounding for bare generic ctor/destructure calls in
the same-module `env.get`-miss path; explicit-args category (full arity);
first-wins retirement with located ambiguity; three new located diagnostics.

**Out of scope:**

- Prefix-pinned ctor type args (`Ok[i64]` meaning `Ok[i64 'E]`) — deferred with
  R7; needs its own probe.
- Cross-module header ambiguity — S10's territory (verified structurally
  separate); this slice's ties are same-module mints of one header.
- Any IR/lowering/emit change (NFR-1).
- S5 declared-overload tier policy and S2-9 member dispatch (NFR-2).

## Advisory solution approach

Not binding; the implementation phase adjudicates.

1. Restructure the `env.get`-miss branch (`terms.rs:889-927`) so that after
   `mint_fallback_candidates` returns, candidates flow through a compatibility
   filter conditioned on a call-site-derived θ (R-1/R-3) rather than the
   unconditional `[only]` take (`terms.rs:950-991`). The zero-candidate arm
   splits: known-header-but-unbound → R-5 unbound-parameter error; no-header →
   unchanged `unknown_word_error` (`terms.rs:923`). The S8b span-keyed pin
   (`terms.rs:974-988`: `is_generated_enum_word` →
   `poly.builtin_overloads.insert(span, only.symbol)`) sits inside this arm and
   must survive the restructuring — kept in the compatibility-conditioned
   selection path or relocated with it — or G6's non-regression breaks for bare
   generated enum words at single-instantiation sites (lowering's bare-key map
   is last-write-wins across instantiations, `src/ir/layout.rs`).
2. Derive θ at the resolve loop from R-2's three inputs in precedence order. The
   consumer-driven step mirrors S2-9's obligation style (constraint lands at the
   resolve loop); literal-driven step reads operand `Type`s already at the call
   site and discarded today.
3. Replace `select_overload_fallback_sourced`'s `matching.first()` fallback
   (`builtins.rs:178-181`; the `None` arm already yields
   `OverloadPick::Ambiguous`) — for this path only — with an ambiguity return
   that lists the tied types sorted by rendered string. Keep the S5 tier-1
   own-module check. Retiring first() inverts two load-bearing doc comments that
   must be updated alongside the code: `mint_fallback_candidates`'s "this
   fallback must not invent a stricter rule than a present `env` entry would
   have had" (`terms.rs:2000-2003` — R-3's sole-incompatible-mint rejection is
   precisely such a stricter rule, now deliberate) and
   `select_overload_fallback_sourced`'s first-match rationale
   (`builtins.rs:146-163`).
4. Widen `poly_call_takes_type_args` (`terms.rs:1314-1348`) with a new admitted
   category: a bare generic ctor/destructure name with a matching header, full
   arity validated. S8c does not pre-build this — the gate still admits exactly
   two categories at `486eda4` (round-2 verified).
5. Preserve monomorph identity via the existing `(word, θ)` mint keying (P7.S3t).

6. Treat S8c's machinery as a shape-template, not a shortcut:
   `compose_member_theta` (`poly.rs:9415`, θ from a seeded substitution with a
   fail-closed tail at `:9462`) and `check_concrete_member_site_slots`
   (`poly.rs:9506`) are the closest existing analogs to R-1/R-2's θ derivation
   and R-3's compatibility filtering, but they hook bound dispatch only — no
   obligation record exists on the ctor path, nothing is reusable as-is, and
   all four S11 capabilities remain to build.

## Codebase map

Every file touched, anchored path:line + symbol (fresh lines re-verified at
`486eda4`; symbols located by grep, not trusted from the draft; `parser.rs`,
`ast.rs`, and `check.rs` are byte-identical to the dp-round tree):

- `src/check/terms.rs`
  - `Call` arm `env.get`-miss branch, `terms.rs:889-927` (`None => match
    env.get(name)` at `:889`, `mint_fallback_candidates` call at `:899`,
    `from_fallback = true` at `:900`, `unknown_word_error` at `:923`) —
    restructured for R-1/R-3/R-4/R-5/R-7.
  - Chosen `[only]` arm, `terms.rs:950-991` (`let chosen = match candidates` at
    `:950`) — no longer an unconditional take; compatibility-conditioned. The
    S8b span-keyed pin, `terms.rs:974-988` (`is_generated_enum_word` →
    `poly.builtin_overloads.insert(span, only.symbol)` at `:987`), sits inside
    this arm and must survive or be relocated with it (G6 gate).
  - S9 single-candidate pre-guard, `terms.rs:939-940`
    (`bare_generated_word_own_module_grounding` call) — shares the outer `Call`
    arm; collateral-change risk covered by G8.
  - `fn poly_call_takes_type_args`, `terms.rs:1314-1348` — new admitted category
    (R-6); gate caller at `terms.rs:199-204`.
  - `fn bare_generated_word_own_module_grounding`, `terms.rs:1483` — same-module
    grounding entry; extended/consulted for R-1.
  - `fn foreign_single_candidate_grounding`, `terms.rs:1690` — **contract
    untouched** (NFR-1; entry condition still "candidates already collapsed to
    `[only]`", doc `:1676-1688`; untouched by S8b/S8c); restructuring routes
    around it.
  - `fn mint_fallback_candidates`, `terms.rs:2024` — candidate source; unchanged
    behavior, consumed by the new compatibility filter. Its doc's first-wins
    sentence ("this fallback must not invent a stricter rule…", `:2000-2003`)
    is inverted by R-3/R-4 and updated alongside the code.
- `src/check/builtins.rs`
  - `fn select_overload_fallback_sourced`, `builtins.rs:164-183` — tier-1
    own-module check at `:176`, `matching.first()` fallback at `:178-181` (the
    `None` arm already yields `OverloadPick::Ambiguous`): gains the ambiguity
    return for this path (R-4). Doc rationale `:146-163` updated alongside.
    Sole caller: `terms.rs:1006`, guarded by `from_fallback`.
- `src/check/poly.rs` — S8b/S8c machinery, read-only shape-template (no hook):
  - `fn compose_member_theta`, `poly.rs:9415` (call site `:9347`) — closest
    analog for R-1/R-2's θ derivation; fail-closed tail at `:9462`.
  - `fn fence_quotation_literal_slot`, `poly.rs:9480`.
  - `fn check_concrete_member_site_slots`, `poly.rs:9506` — closest analog for
    R-3's compatibility filtering.
  - `fn member_unbound_variable_error`, `poly.rs:9632`; `fn
    first_unbound_sig_var`, `poly.rs:9654`.
- `src/check/check.rs`
  - env build, `check.rs:579-603` (declaration at `:579`,
    `struct_generated_sigs` population loop at `:586`) — mechanism reference
    for R-1; unchanged at `486eda4`.
- `src/parser.rs`
  - `fn resolve_type_or_apply`, `parser.rs:7294` (eager-mint site;
    `find_struct` arm ~`:7326`, `find_enum` arm `:7348-7356` with
    `instantiate_enum` at `:7354`) — read-only reference; touched only if R-6's
    explicit args need parse support for the ctor-name category.
- `src/ir/layout.rs` — read-only reference for the S8b pin's rationale (the
  bare-key map note at `:556-557`); must not change (NFR-1).
- New: `tests/phase7b_slice11.rs` — G1–G8 goldens + units.

## Open questions and risks

- **Prefix-pinned ctor type args** (`Ok[i64]` → `Ok[i64 'E]` with `'E` from
  context) — deferred with R7; needs its own probe if ever wanted.
- **dp_e2 no-consumer shape under R-6**: with explicit args the construction is
  fully concrete and reaches the ordinary forgetting check. Record the observed
  diagnostic in the probe appendix during implementation.
- **Cross-module ambiguity boundary**: verified structurally separate from S10,
  but the restructuring shares the outer `Call` arm; risk of collateral change to
  the S10 fall-through. Mitigation: G8 diffs S10 stderr byte-for-byte (baseline).
- **Consumer-driven re-grounding cost**: obligation-style re-grounding at the
  resolve loop is unbuilt for this path; risk it interacts with the two-pass
  split. Mitigation: G5 pins the map-driven case end to end.
- **S8b pin survival**: the span-keyed pin (`terms.rs:974-988`) sits inside the
  `[only]` arm this slice restructures; dropping it silently re-types
  single-instantiation bare generated-enum-word sites at lowering (the bare-key
  map is last-write-wins across instantiations). Mitigation: keep the pin in
  the compatibility-conditioned path or relocate it with the selection logic;
  G6's dp_a/dp_b/dp_f shapes exercise exactly this channel.
- **Maintainer inclined-to-B in chat** (brief rulings 260907): confirm R1–R7 at
  spec review round 1 before implementation.

## Phased delivery plan

**Phase 1 — checker grounding core.** Obligation-style re-grounding (R-1/R-2),
compatibility-conditioned selection (R-3), first-wins retirement with located
ambiguity (R-4), the three new located diagnostics (R-5), zero-mint/undefined
split (R-7). Touches the `env.get`-miss branch (`terms.rs:889-927`), the chosen
`[only]` arm (`terms.rs:950-991`), and `builtins.rs:164-183`. The S8b span-keyed
pin (`terms.rs:974-988`) survives the restructuring — preserved in the
compatibility-conditioned path or relocated with it (G6 gate). Retiring first()
inverts two load-bearing docs — `mint_fallback_candidates`' stricter-rule
sentence (`terms.rs:2000-2003`) and `select_overload_fallback_sourced`'s
first-match rationale (`builtins.rs:146-163`) — both updated alongside the code.
Goldens G1, G2, G3, G7 + core units. Preserves R-8.

**Phase 2 — explicit-args category.** Widen `poly_call_takes_type_args`
(`terms.rs:1314-1348`) for the bare-ctor category with full-arity validation
(R-6); the gate still admits exactly two categories at `486eda4` — S8c does not
pre-build this. Parser support if needed (`parser.rs:7294`). Goldens G4, G5 +
units.

**Phase 3 — non-regression sweep + docs.** Baseline diff against
`probes/dp_baseline.md` (holds byte-for-byte at `486eda4`, round-2 re-run) and
the S10 suite (G6, G8, NFR-2/NFR-4); NFR-1 judged behaviorally — no behavioral
IR/lowering/emit change relative to `486eda4`. Clean up compiler-written ELF
artifacts under `probes/` after probe rounds (not gitignored). Roadmap entry;
record dp_e2's observed diagnostic in the probe appendix. Re-run growth signals
(R6, CLAUDE.md) on every touched file.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Checker grounding core",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Explicit-args category",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Non-regression sweep and docs",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
