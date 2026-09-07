# P7b.S11 — per-call-site grounding for bare generic constructors

> Companion frozen docs: [slice11-brief](./slice11-brief.md) (problem, mechanism,
> rulings R1–R7, proposed goldens), [slice11-probes](./slice11-probes.md) (verbatim
> dp_a–dp_h round with Q1–Q3 analysis), byte-exact stderr baseline
> `probes/dp_baseline.md`, fixtures `probes/dp_*.sth`. Sits beside
> [slice10-spec](./slice10-spec.md) (the cross-module header-ambiguity sibling; this
> slice is the same-module counterpart and is verified structurally separate from it).
> Base `7404a71` (main; dp round ran at this HEAD, suite green). Invocation
> `cargo run -q -- run probes/<name>.sth --manifest tests/fixtures/sooth.pkg`.

## Why

Bare generic constructor/destructure calls (`Ok`, `Err`, and enum/struct
generated words with a free type parameter) have **no per-call-site grounding**.
The env is built once from parse-time eager mints only (`check.rs:586`); a bare
ctor call misses `env` and falls to `mint_fallback_candidates` (`terms.rs:2010`),
which scans the whole-module monomorph registry for a name match with no
call-site information. This produces four measured defects (probe round, verbatim
in slice11-probes):

1. **Zero-mint misdiagnostic (dp_c).** `1 Ok drop` with no concrete `Res[...]`
   application anywhere in the module → the generic `unknown word` error
   (`terms.rs:923`), indistinguishable from a genuinely undefined name and blaming
   the wrong word rather than the unbound parameter `'E`.
2. **Wrong-mint forcing (dp_d).** A sole existing mint is taken unconditionally
   (`terms.rs:951`, the `[only]` arm) regardless of fit. `1 mkok Ok drop` forces
   the inner `Ok` onto the unrelated `Res[i64 i64]` mint from `mkok`'s signature
   and fails later with a far-away operand mismatch that never mentions grounding.
   Rejection is correct; the mechanism reaching it is accidental.
3. **Silent first-wins on ties (dp_g2/dp_g3).** With 2+ mints,
   `select_overload_fallback_sourced` (`builtins.rs:164-183`) falls back to
   `matching.first()` — declaration order — with no ambiguity check. Two programs
   differing only in unrelated declaration order construct different runtime types
   from the same bare call, silently. **A correctness gap, not a diagnostics gap.**
4. **No explicit-args category (dp_e/dp_e2).** `Ok[i64 i64]` is rejected at
   `poly_call_takes_type_args` (`terms.rs:1300-1335`): only user-declared poly
   words and trait members admit type args. No escape-hatch spelling exists,
   independent of grounding; the consumer is irrelevant (dp_e2 byte-identical).

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
  `poly_call_takes_type_args` (`terms.rs:1300`) admits a new category: a bare
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

- **NFR-1 (checker-stage fence — R6 of brief).** No IR/lowering/emit changes. All
  edits in `src/check/`. The `foreign_single_candidate_grounding` contract
  (`terms.rs:1676`) is untouched; the `env.get`-miss branch is restructured around
  it, not through it.
- **NFR-2 (behavioral fence — R5 of brief).** No correct program regresses.
  dp_a/dp_b/dp_f behaviors byte-identical to baseline. S10's diagnostics
  byte-unchanged (baseline diff). S5's declared-overload tier policy and S2-9's
  member dispatch untouched.
- **NFR-3 (determinism).** Ambiguity text is order-stable across declaration
  orders (candidate types sorted by rendered string). No run-count/ratio
  assertions; determinism pinned by byte-exact text across orderings.
- **NFR-4 (no cross-module churn).** Cross-module header ambiguity remains S10's
  territory; ties in this slice are same-module mints of one header (verified
  structurally separate: the fall-through `terms.rs:888-928` is upstream of and
  distinct from `foreign_single_candidate_grounding`).

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

1. Restructure the `env.get`-miss branch (`terms.rs:888-928`) so that after
   `mint_fallback_candidates` returns, candidates flow through a compatibility
   filter conditioned on a call-site-derived θ (R-1/R-3) rather than the
   unconditional `[only]` take at `terms.rs:951`. The zero-candidate arm splits:
   known-header-but-unbound → R-5 unbound-parameter error; no-header → unchanged
   `unknown_word_error`.
2. Derive θ at the resolve loop from R-2's three inputs in precedence order. The
   consumer-driven step mirrors S2-9's obligation style (constraint lands at the
   resolve loop); literal-driven step reads operand `Type`s already at the call
   site and discarded today.
3. Replace `select_overload_fallback_sourced`'s `matching.first()` fallback
   (`builtins.rs:180`) — for this path only — with an ambiguity return that lists
   the tied types sorted by rendered string. Keep the S5 tier-1 own-module check.
4. Widen `poly_call_takes_type_args` (`terms.rs:1300`) with a new admitted
   category: a bare generic ctor/destructure name with a matching header, full
   arity validated.
5. Preserve monomorph identity via the existing `(word, θ)` mint keying (P7.S3t).

## Codebase map

Every file touched, anchored path:line + symbol (verified against HEAD `7404a71`;
line numbers drift ±a few from brief citations, symbols confirmed):

- `src/check/terms.rs`
  - `Call` arm `env.get`-miss branch, `terms.rs:888-928` (candidate derivation,
    `mint_fallback_candidates` call, `unknown_word_error` at ~923) — restructured
    for R-1/R-3/R-4/R-5/R-7.
  - `[only]` single-candidate arm, ~`terms.rs:951` — no longer an unconditional
    take; compatibility-conditioned.
  - `fn poly_call_takes_type_args`, `terms.rs:1300` — new admitted category (R-6).
  - `fn bare_generated_word_own_module_grounding`, `terms.rs:1469` — same-module
    grounding entry; extended/consulted for R-1.
  - `fn foreign_single_candidate_grounding`, `terms.rs:1676` — **contract
    untouched** (NFR-1); restructuring routes around it.
  - `fn mint_fallback_candidates`, `terms.rs:2010` — candidate source; unchanged
    behavior, consumed by the new compatibility filter.
- `src/check/builtins.rs`
  - `fn select_overload_fallback_sourced`, `builtins.rs:164-183` — the
    `matching.first()` fallback (`:180`) gains an ambiguity return for this path
    (R-4).
- `src/parser.rs`
  - `fn resolve_type_or_apply`, `parser.rs:7294` (eager-mint site, `find_enum` arm
    ~7345) — read-only reference for the mechanism; touched only if R-6's explicit
    args need parse support for the ctor-name category.
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
- **Maintainer inclined-to-B in chat** (brief rulings 260907): confirm R1–R7 at
  spec review round 1 before implementation.

## Phased delivery plan

**Phase 1 — checker grounding core.** Obligation-style re-grounding (R-1/R-2),
compatibility-conditioned selection (R-3), first-wins retirement with located
ambiguity (R-4), the three new located diagnostics (R-5), zero-mint/undefined
split (R-7). Touches `terms.rs:888-928`, the `[only]` arm, and
`builtins.rs:164-183`. Goldens G1, G2, G3, G7 + core units. Preserves R-8.

**Phase 2 — explicit-args category.** Widen `poly_call_takes_type_args`
(`terms.rs:1300`) for the bare-ctor category with full-arity validation (R-6).
Parser support if needed (`parser.rs:7294`). Goldens G4, G5 + units.

**Phase 3 — non-regression sweep + docs.** Baseline diff against
`probes/dp_baseline.md` and the S10 suite (G6, G8, NFR-2/NFR-4). Roadmap entry;
record dp_e2's observed diagnostic in the probe appendix. Re-run growth signals
(R6, CLAUDE.md) on every touched file.

## Phases (JSON)

```json
[
  {
    "phase": 1,
    "focus": "Checker grounding core: obligation-style re-grounding, compatibility-conditioned selection, first-wins retirement, three new located diagnostics, zero-mint/undefined split",
    "effort": "L",
    "difficulty": "high"
  },
  {
    "phase": 2,
    "focus": "Explicit-args category: widen poly_call_takes_type_args for bare ctors with full-arity validation",
    "effort": "M",
    "difficulty": "medium"
  },
  {
    "phase": 3,
    "focus": "Non-regression sweep against dp_baseline and S10 suite, roadmap + probe-appendix docs, growth-signal re-run",
    "effort": "S",
    "difficulty": "low"
  }
]
```
