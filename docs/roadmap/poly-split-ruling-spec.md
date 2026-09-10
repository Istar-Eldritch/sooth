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

### Gate — D1: RULED — band extraction + core holdout (A + C), 260910

Ruled by the maintainer on the Phase 1 dossier
([poly-split-ruling-audit.md](./poly-split-ruling-audit.md) §4–§6). The pre-committed
rule ("both sides < 2 firing signals") was tested to exhaustion and is unsatisfiable
for this file: three single-seam probes and a full six-band job partition (48% of
non-test mass moved, every band landing at 0/weak) each left the remainder firing all
three signals — the walk core is one interlocking mass that no job boundary separates
(§6.9). The ruled outcome combines the ticket's two named exits: **land the
measured-clean bands under supervision** (D2, amended rule below) **and record the
irreducible core as a deliberate holdout** (D3), with the core's tests moving to a
sibling file (C).

**Amended D2 rule (supersedes "both sides < 2"):** a band qualifies to land iff the
dossier measured it clean post-cut (0 or weak firing signals on the band side), its
move neither splits nor crosses either recursion SCC, and its wiring stays inside the
proven recipe (visibility widenings + re-exports; zero behavior change). The core's
persistent 3-firing count is accepted and recorded as an explicit holdout with reopen
conditions — it is not a failed split. Churn stays excluded: a band that cannot land
green, or that lands with new recursion-crossing coupling, does not land. The ruled
band set: unify, instantiate, trait, overload, crosscall, ground (dossier §6.1–§6.6
recipes), plus a construction/elimination band **only if** it first passes the same
revert-protocol measurement (candidate: `poly_construction_header`,
`poly_bind_construction_arg`, `poly_destructure_generic` (verified leaf),
`poly_construct_generic` and construction-local helpers).

### Phase 2a — execute the band extraction (D2, as ruled)

Land the ruled band set into `src/check/poly/` (`mod r#trait;` spelling for
`trait.rs`), in the proven order, **green after every band** (`cargo check`,
`cargo clippy -- -D warnings`, test target compiles — the tree is never left red
between bands):

- Follow the dossier's per-band recipes: §4 protocol with §6.1–§6.6 move lists,
  visibility widenings, and re-export mechanics (the corrected counts: unify needs 4
  widenings, not 2; a `pub(super)` glob re-export fails outright when nothing is
  visible enough, E0365; a struct and its impl move as one cut unit).
- Relocate each band's attributed unit tests beside it, per the §6.7 attribution map:
  the band's test fns move verbatim into its own inline `#[cfg(test)] mod tests`
  (house style), helper fixtures are per-module copies exactly as
  `terms.rs`/`declarations.rs`/`engine.rs` already do (`checked_module` et al.). No
  assertion text changes — imports and visibility only. A test that exercises core
  behavior *through* a band entry point stays with the core; deviations from the
  attribution map are recorded in the phase notes. The one double-attributed test
  (§6.7) gets a recorded judgment.
- The 7th (construction) band: probe under the full revert protocol first; land only
  if measured clean under the amended D2 rule; otherwise skip and record why.
- At phase exit, re-run CLAUDE.md's five signals over the core and every band and
  record the counts in the roadmap entry.

### Phase 2b — the core holdout (D3) + tests-out (C)

- **C, tests-out:** move the core's remaining test region (dossier §6.7: 347 fns /
  ~7,698 lines) verbatim to `src/check/poly/tests.rs`, included from poly.rs as
  `#[cfg(test)] mod tests;` — same attribute, same directory; convention blessed by
  this ruling. Imports adjusted only; no assertion changes.
- **D3, the holdout record:** write the ruling into `docs/roadmap/P7-language-prereqs.md`
  beside the S12/S11 growth records, reflected in the ROADMAP row. It states: the
  core stays whole; the measured facts (dossier §2/§6.7 — walk, construction/
  elimination, copy/borrow gating, dispatch hub, shared plumbing, and their 59
  formatters are one interlocking mass; both probe rounds' evidence); the two
  rejected shapes named with their standing reasons (`poly/diagnostics.rs`
  layer-shaped with no precedent in this checker; `poly/eliminator.rs` crosses the
  recursion cluster); and **what would reopen the question** — import divergence
  starting to fire (`RefCell` has already drifted to the instantiation cluster), a
  new responsibility axis appearing in the core, or the core's own count dropping.
- The record states the ticket's exit criterion is satisfied by the combination: a
  supervised split of the measured-clean bands plus a written holdout ruling for the
  core, with the post-landing signal counts for every file.

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
      "focus": "Execute the ruled A+C outcome: land the six measured-clean bands (unify, instantiate, trait, overload, crosscall, ground) into src/check/poly/ per the dossier recipes, green after every band, plus a construction band only if it passes the revert-protocol measurement; relocate attributed unit tests beside their bands verbatim with per-module helper copies; move the core's remaining tests to src/check/poly/tests.rs behind #[cfg(test)] mod tests; write the core holdout ruling beside the S12/S11 records with post-landing signal counts and reopen conditions; update the ROADMAP row; hold the green gate (fmt, clippy -D warnings, test, phase7/phase7b goldens)",
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
