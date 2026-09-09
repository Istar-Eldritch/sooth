# The poly.rs ruling (spec)

SOO-32. `src/check/poly.rs` carries standing split-signal debt: 3 of CLAUDE.md's 5
growth signals have fired at every recorded phase exit since P7.S12, and both
candidate splits probed so far (`poly/diagnostics.rs`, `poly/eliminator.rs`) were
rejected. The ticket's own threshold — "if the same 3/5 persists after two more
consecutive exits with no split landing, rule on it explicitly" — is met many times
over. This work makes the ruling: **either land a responsibility-shaped split whose
cut reduces the firing-signal count on both resulting files, or write a durable
holdout ruling into the roadmap saying the file stays whole and why.** The decision
is made on fresh measurements taken this cycle, not on the recorded number (the
P7b.S8 precedent: the design is measured, not argued). Discovery lives in
[poly-split-ruling-brief](./poly-split-ruling-brief.md).

## Goal

Close the ticket's exit criterion once, either way, and record the outcome beside
the S12/S11 growth records in `docs/roadmap/P7-language-prereqs.md` (with the ROADMAP
row reflecting it). No diagnostic text moves and no golden changes: the work is
structural or documentary only.

## Phases

The spec pre-commits the audit and the decision rule. It does **not** pre-commit the
outcome (D1), the seam (D2), or the holdout's exact prose (D3): those are settled at
the gate, on the audit's evidence.

### Phase 1 — the measured audit (no behavior change, no commit)

Produce a dossier of fresh measurements against soo-32 @ HEAD. This phase changes no
committed file. Any structural experiment is a **disposable no-commit probe** on a
clean tree, reverted with `git checkout` (the standing probe convention). The audit
delivers:

1. **Size, split test from non-test.** `wc -l` on `poly.rs`; the `mod tests` boundary
   (poly.rs:12830 at discovery) re-located; non-test source lines counted separately.
   The ruling's size argument must separate the two: unit tests travel with the code
   they cover, so they do not count toward a stage-file's complexity.

2. **The three firing signals, re-measured.** (a) *Does X and Y and Z* — the distinct
   jobs (signature/grounding, the abstract walk, instantiation, unification/
   substitution), each anchored to named functions. (b) *High/low-level mixed* — the
   `*_error` formatter count and where it interleaves with walk logic. (c) *Functions
   that never call each other* — the top-level function clusters with no call edges
   into the walk cluster. Report each as a current number, not the recorded one.

3. **The two quiet signals, re-measured.** Import divergence (`RefCell`, `Ordering`,
   `GenericTypes` site distributions) and forced circularity (the
   `poly_call_term → poly_eliminator_call → poly_walk → poly_call_term` cycle,
   crossing `poly_destructure_generic`). Confirm whether either now fires.

4. **Candidate seams, each measured the same way.** For every candidate leaf seam
   — the unification/substitution cluster (`unify_poly_input`, `apply_subst`, the
   `Subst`/`CrossGround` machinery), the instantiation cluster
   (`discover_transitive_instantiations`), the signature-grounding region — measure,
   via a disposable probe, what the firing-signal count would be **on each side** of
   the cut. A seam qualifies for D2 only if it drops both sides below 2 firing
   signals. The two rejected shapes (`poly/diagnostics.rs`, `poly/eliminator.rs`) are
   **out of bounds** and are not re-probed.

The audit's output is a dossier, not a code change. Nothing from Phase 1 is committed.

### Gate — D1: split or holdout

A genuine user ruling, taken after the audit, on the audit's dossier. The evidence
selects the branch; the outcome is not pre-committed:

- **Take the split branch (D2)** iff the audit found **at least one qualifying seam**:
  a responsibility-shaped cut whose measured signal count is **< 2 on both sides**,
  that also cuts (not increases) the recursion cycle, and that relocates unit tests
  beside their stage code without touching any assertion.
- **Take the holdout branch (D3)** iff **no seam qualifies**: every probed cut leaves
  one side at ≥ 2 firing signals, or buys more coupling (a crossed call cycle, a new
  import spread) than lines. This is the standing outcome unless the audit overturns
  it with a measurement.

If the split branch is available, the seam (D2) is confirmed at this same gate from
the qualifying candidates.

### Phase 2a — execute the split (only if D1 = split)

Land the seam chosen at the gate:

- Move the responsibility cluster to its own file under `src/check/poly/` (module
  wiring adjusted, `use super::*` or explicit re-exports as the existing checker
  files do).
- Relocate the covering unit tests beside the moved code. No assertion text changes;
  the goldens (`tests/phase7_*.rs`, `tests/phase7b_*.rs`) are untouched.
- Re-run CLAUDE.md's five signals over **both** resulting files at phase exit and
  record the new counts. A split that leaves either side at 2+ firing signals is
  churn, not a split, and must not land.
- Record the split beside the S12/S11 growth records in
  `docs/roadmap/P7-language-prereqs.md`: the seam, the pre/post signal counts on both
  sides, and why this cut qualified where the two rejected shapes did not. Reflect it
  in the ROADMAP row.

### Phase 2b — record the holdout (only if D1 = holdout)

Write the durable holdout ruling into `docs/roadmap/P7-language-prereqs.md` beside
the S12/S11 growth records, and reflect it in the ROADMAP row. The ruling states:

- The file stays whole.
- The current measured signal counts (firing and quiet), test vs non-test size.
- The coupling facts that make every probed cut cost more than it buys: for each
  audited candidate seam, the side that stays at ≥ 2 signals or the coupling the cut
  crosses. The two rejected shapes are named with their standing reasons
  (`poly/diagnostics.rs` layer-shaped with no precedent in this checker;
  `poly/eliminator.rs` crosses the recursion cluster).
- **What would reopen the question**: the concrete change that would make a cut
  qualify (e.g. a fourth responsibility axis appearing, or import divergence starting
  to fire), so a future exit knows when to re-run the audit rather than re-cite this
  ruling.

This closes the ticket's exit criterion. It is documentary: no code moves.

### Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "The measured audit of src/check/poly.rs: test/non-test size split, the three firing and two quiet growth signals re-measured with current numbers, and every candidate seam (unification/substitution, instantiation, signature-grounding) probed for its post-cut signal count on both sides via disposable no-commit probes reverted with git checkout; the dossier is produced, nothing is committed",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "The D1 gate: present the audit dossier and the user rules split or holdout on the pre-committed evidence criteria (a qualifying seam drops both sides below 2 firing signals and cuts, not crosses, the recursion cycle); if split, the seam (D2) is confirmed from the qualifying candidates at this same gate",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Execute the ruling: if split, land the chosen seam under src/check/poly/ with unit tests relocated beside their code and no assertion touched, re-run all five signals over both resulting files (2+ on either side means the split does not land), and record the seam with pre/post counts beside the S12/S11 growth records; if holdout, write the durable ruling with the coupling facts and the reopen condition; either way update the ROADMAP row, run the growth re-check, and hold the green gate (fmt, clippy -D warnings, test, phase7/phase7b goldens)",
      "effort": "L",
      "difficulty": "hard"
    }
  ]
}
```

## Green gate (either branch)

`cargo fmt --check && cargo clippy -- -D warnings && cargo test` stays green, plus the
P7/P7b golden suites (`tests/phase7_*.rs`, `tests/phase7b_*.rs`). On the holdout
branch this is unchanged by construction (docs-only). On the split branch it must hold
after the move: a relocation touches no assertion, so any red is a wiring error.

## Out of bounds

- Re-proposing `poly/diagnostics.rs` or `poly/eliminator.rs` in either shape.
- Any behavior change: diagnostics text and goldens must not move.
- Splitting `parser.rs` (same standing size fact, no ticket).
- Committing anything from the Phase 1 audit; probes are disposable and reverted via
  `git checkout` on a clean tree.
