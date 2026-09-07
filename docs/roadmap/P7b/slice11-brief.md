# P7b.S11 brief — per-call-site grounding for bare generic constructors

- Date: 260907. Base: `7404a71` (main; the dp probe round ran at this HEAD, suite green).
- Provenance: carved out of the P7b semantics walkthrough (maintainer chat, 260905–260907).
  The trigger was a `map`-then-`drop` pipeline over a two-channel type failing with
  ``unknown word `Ok` ``; the probe round traced the real mechanism and widened the defect
  list well past the triggering shape. Landing ahead of P9.S2 keeps the alloc-layer brief
  free of checker-grounding concerns; this slice is checker-stage only and has no forcing
  dependency on P9.
- Sources: probe round [slice11-probes](./slice11-probes.md) (verbatim, dp_a–dp_h plus the
  Q1–Q3 analysis), byte-exact stderr baseline `probes/dp_baseline.md`, fixtures
  `probes/dp_*.sth`, working invocation
  `cargo run -q -- run probes/<name>.sth --manifest tests/fixtures/sooth.pkg`.

## Problem

Four defects in the bare generic-ctor/destructure call path (evidence in slice11-probes):

1. **Zero-mint misdiagnostic (dp_c).** `1 Ok drop` with no concrete `Res[...]` application
   anywhere in the module → the generic `unknown word` error (`terms.rs:923`),
   indistinguishable from a genuinely undefined name and blaming the wrong word.
2. **Wrong-mint forcing (dp_d).** A sole existing mint is taken unconditionally
   (`terms.rs:951`) regardless of fit. The nested shape `1 mkok Ok drop` forces the inner
   `Ok` onto the unrelated `Res[i64 i64]` mint from `mkok`'s signature and fails later with
   an operand mismatch that never mentions grounding. Rejection is correct; the mechanism
   is accidental.
3. **Silent first-wins on ties (dp_g2/dp_g3).** With 2+ mints,
   `select_overload_fallback_sourced` (`builtins.rs:164-183`) falls back to
   `matching.first()` — declaration order — with no ambiguity check (documented as
   deliberate, `terms.rs:1985-1989`). Two programs differing only in unrelated declaration
   order construct different runtime types from the same bare call, silently. **This is a
   correctness gap, not a diagnostics gap.**
4. **No explicit-args category (dp_e/dp_e2).** `Ok[i64 i64]` is rejected at
   `poly_call_takes_type_args` (`terms.rs:1300-1335`): only user-declared poly words and
   trait members admit type args. No escape-hatch spelling exists, independent of
   grounding; the consumer is irrelevant (dp_e2 byte-identical).

## Adjudicated mechanism (probe round)

- There is **no per-call-site grounding** for bare ctor calls. Grounding is a parse-time,
  whole-module, incidental side effect: any concrete application (`Res[i64 i64]`) spelled
  in any signature — used or not, called or not — mints the monomorph into the shared
  registry (`resolve_type_or_apply`, `parser.rs:7294`, find_enum arm ~7345).
- The env is built once, before any body is checked, from eager mints only
  (`check.rs:586`). A bare ctor call misses `env` and falls to `mint_fallback_candidates`
  (`terms.rs:2010`), which re-derives candidates by scanning the current whole extended
  type-slice registry for a name match — no call-site information of any kind.
- Branch outcomes: 0 mints → `unknown_word_error` (`terms.rs:923`); exactly 1 →
  unconditional take (`terms.rs:951`); 2+ → tier-1 own-module check then `first()`
  (`builtins.rs:164-183`).
- dp_f/dp_h: minting is whole-module and declaration-order-independent (an unused sibling
  declared *after* `main` still grounds the call); only the tie-break is order-dependent.
- Literal operand types (`1` → i64) are available at the call site and today discarded.
- **No S9/S10 collision**: the fall-through (`terms.rs:888-928`) is upstream of and
  structurally separate from `foreign_single_candidate_grounding` (`terms.rs:1676`), which
  runs only when candidates have already collapsed to `[only]`. Baseline frozen in
  `probes/dp_baseline.md`.

## Rulings (proposed 260907; maintainer inclined-to-B in chat; confirm at spec review round 1)

- **R1 (direction).** Build per-call-site grounding for bare generic ctor/destructure
  calls — not a diagnostics-only fix. The silent wrong-pick (defect 3) is a correctness
  gap; sharpened errors alone would at best turn it into a rejection.
- **R2 (tie-break).** Explicit type args > single compatible candidate > located ambiguity
  error. First-wins is retired. The ambiguity error lists the tied candidate types sorted
  by rendered type string, so the text is stable across declaration orders (S10 GA
  discipline).
- **R3 (grounding inputs, precedence order).** Explicit type args (full arity) >
  consumer-driven re-grounding (S2-9 obligation style: the consumer's constraint lands at
  the resolve loop) > literal-driven partial inference (operand types pin leading
  parameters). Parameters still unbound after all inputs → located "unbound type
  parameter" error naming the parameter, its position, and the remedy (explicit args or a
  concretely-typed consumer).
- **R4 (mints are candidates, not verdicts).** An existing mint participates in
  compatibility selection like any other candidate; a sole *incompatible* mint → located
  grounding error naming expected type vs mint (dp_d upgrades from a far-away operand
  mismatch to a grounding-site error).
- **R5 (behavioral fence).** No correct program regresses: dp_a/dp_b/dp_f behaviors
  byte-identical; the silently-wrong shapes become either correctly grounded or located
  errors; S10's diagnostics byte-unchanged (baseline diff); S5's declared-overload tier
  policy and S2-9's member dispatch untouched.
- **R6 (machinery fence).** Checker-stage only; no IR/lowering changes; monomorph
  identity/dedup invariants preserved (one symbol per `(word, θ)`, canonical sorting per
  P7.S3t); the `env.get`-miss branch is restructured without touching
  `foreign_single_candidate_grounding`'s contract.
- **R7 (arity).** Explicit args on bare ctors require full arity; prefix pinning
  (impl-target style, `for Res[i64]`) is out of scope, recorded as an open question.

## Goldens (proposed; `tests/phase7b_slice11.rs`)

| Golden | Shape | Behavior |
| --- | --- | --- |
| G1 | dp_c (`1 Ok drop`) | located unbound-parameter error naming `'E`, byte-exact; not `unknown word` |
| G2 | dp_d (nested, sole wrong mint) | located grounding error naming expected vs mint, byte-exact |
| G3 | dp_g2 + dp_g3 (tied mints, both orders) | located ambiguity error, byte-identical across the two declaration orders |
| G4 | dp_e (`1 Ok[i64 i64] drop`) | accepted, runs clean |
| G5 | `1 Ok [ 1 sub ] map[i64 i64 i64] drop` | accepted via consumer-driven grounding (map's args pin the channels), runs, expected output |
| G6 | dp_a / dp_b / dp_f | behavior byte-identical to baseline (non-regression) |
| G7 | genuinely undefined ctor name | `unknown word` byte-unchanged (zero-mints-of-a-known-header must be distinguishable from undefined-name) |
| G8 | S10 shapes (`probes/dp_baseline.md`) | stderr byte-identical; slice10 goldens stay green |

Plus units beside each changed site (`terms.rs` gate + fall-through, `builtins.rs`
fallback, `parser.rs` if the args category needs parse support): target ~12, named
`thing_condition_expected`.

## Open questions

- Prefix-pinned ctor type args (`Ok[i64]` meaning `Ok[i64 'E]` with `'E` from context?) —
  deferred with R7; needs its own probe if ever wanted.
- Cross-module header ambiguity is deliberately out: ties here are same-module mints of
  one header; the cross-module dimension remains S10's territory (verified structurally
  separate).
- dp_e2's no-consumer shape under B: with explicit args the construction is fully
  concrete; the value then reaches the ordinary forgetting check. Record the observed
  diagnostic in the probe appendix during implementation.
