# The poly.rs ruling (reference)

SOO-32. Status: **done** — ruled A+C, landed at `0c6aabc`.

## What and why

`src/check/poly.rs` carried standing split-signal debt: 3 of CLAUDE.md's 5 growth
signals fired at every recorded phase exit since P7.S12, yet both splits probed
earlier (`poly/diagnostics.rs`, `poly/eliminator.rs`) were rejected. The ticket's own
threshold ("if the same 3/5 persists after two more exits with no split landing, rule
explicitly") was met many times over. This work made the ruling, on fresh
measurements taken this cycle (the P7b.S8 precedent: the design is measured, not
argued).

The pre-committed decision rule was **"a seam qualifies only if it drops both resulting
sides below 2 firing signals."** The Phase 1 audit tested that rule to exhaustion and
found it **unsatisfiable for this file**: three single-seam probes and a full six-band
job partition (48% of non-test mass moved, every band landing at 0/weak) each left the
remainder firing all three signals. The walk core is one interlocking mass that no job
boundary separates — the walk hub `poly_call_term` calls directly into
construction/elimination, copy/borrow gating, and dispatch, and the ~59 formatters
serve them all (audit §6.9).

## The ruling (D1, 260910)

Combine the ticket's two named exits rather than pick one:

- **A — land the measured-clean bands** under supervision, under an amended D2 rule.
- **C — move the core's tests** to a sibling file.
- **D3 — record the irreducible core as a deliberate holdout**, not a failed split.

**Amended D2 rule (supersedes "both sides < 2"):** a band lands iff (1) the dossier
measured it clean post-cut (0 or weak firing signals on the band side), (2) its move
neither splits nor crosses either recursion SCC, and (3) its wiring stays inside the
proven recipe (visibility widenings + re-exports, zero behavior change). The core's
persistent 3-firing count is accepted and recorded, not chased. Churn stays excluded: a
band that cannot land green, or that lands with new recursion-crossing coupling, does
not land.

Both rejected shapes stay out of bounds with their standing reasons:
`poly/diagnostics.rs` is layer-shaped with no precedent in this checker;
`poly/eliminator.rs` crosses the recursion cluster.

## Reopen conditions

The holdout is revisited if any of these hold (recorded beside the ruling in
`docs/roadmap/P7-language-prereqs.md`):

- Import divergence starts to fire — `RefCell` has already drifted toward the
  instantiation cluster.
- A new responsibility axis appears in the core.
- The core's own firing-signal count drops.

## Implementation

- **Band extraction + core holdout (A+C+D2+D3):** `0c6aabc`
  ("refactor(check): partition poly.rs into seven single-job bands + core holdout").
  Seven single-job bands under `src/check/poly/` (`unify.rs`, `instantiate.rs`,
  `r#trait.rs`, `overload.rs`, `crosscall.rs`, `ground.rs`, `construction.rs`);
  construction qualified under the amended rule via probe-then-land during execution.
  `poly.rs` 23,540 → 5,987 lines (walk core + dispatch hub + gating + shared plumbing).
  Attributed unit tests relocated verbatim beside their bands (80 fns; crosscall's one
  attributed test kept in the core — recorded deviation); the core's remaining tests
  moved to `src/check/poly/tests.rs` behind `#[cfg(test)] mod tests;`. Zero behavior
  change (visibility prefixes + rustfmt reflow only; both recursion SCCs whole and
  sole-resident). Green: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- **Evidence dossier (Phase 1 audit):**
  [poly-split-ruling-audit.md](./poly-split-ruling-audit.md) — the measured
  refutation of the "both sides < 2" rule (§4 single-seam probes, §6 six-band
  partition, §6.9 interlocking-core finding, §6.1–§6.7 band recipes and test
  attribution).
- **Holdout ruling record + reopen conditions:** `docs/roadmap/P7-language-prereqs.md`
  (P7.S13, beside the S12/S11 growth records), reflected in the ROADMAP row.
- **Per-band execution notes / deviations:**
  [poly-split-ruling-phase-notes.md](./poly-split-ruling-phase-notes.md).
