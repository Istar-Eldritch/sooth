# The poly.rs ruling (brief)

SOO-32: `src/check/poly.rs` carries standing split-signal debt — 3 of CLAUDE.md's 5
growth signals have fired at every recorded phase exit since P7.S12 first recorded
them, and both candidate splits probed so far were rejected. The ticket's own
threshold — "if the same 3/5 persists after two more consecutive exits with no split
landing, rule on it explicitly" — is met many times over. This work makes the ruling
the ticket demands: **either land a responsibility-shaped split that passes the
post-split signal re-check with all goldens green, or write a durable holdout ruling
into the roadmap saying the file stays whole and why.** The decision is made on
fresh measurements, not on the recorded number — the P7b.S8 precedent: the design is
measured, not argued.

## Recon (measured against soo-32 @ 9ed2890, 260909)

1. **The file has grown ~50% since the last recorded audit, and nearly half of it
   is tests.** `wc -l` = 23,535, vs 15,799 recorded at P7.S11's exit. Non-test code
   is lines 1–12,829; `mod tests` (poly.rs:12830) runs to 23,535 — 10,705 lines, 45%
   of the file, the CLAUDE.md-conventional unit suite living beside its stage code.
   Any size argument in the ruling must separate the two.

2. **The three firing signals, re-measured today.** (a) *Does X and Y and Z*: at
   least four jobs — signature/grounding helpers, the abstract walk
   (`check_poly_body` poly.rs:980 → `poly_walk` :1096 → `poly_call_term` :2983 →
   `poly_eliminator_call` :4837 → `poly_walk_arms` :5360 → `poly_destructure_generic`
   :6419), instantiation (`discover_transitive_instantiations` :8089), and
   unification/substitution (`unify_poly_input` :10807, `apply_subst` :11278).
   (b) *High- and low-level mixed*: 155 `*_error` message formatters (up from the 65
   recorded at S11) interleaved with walk logic from :2205 onward. (c) *Functions
   that never call each other*: 114 top-level functions in formatter/binding
   clusters with no call edges into the walk cluster.

3. **The two quiet signals, re-measured today.** Import divergence still does not
   fire: `RefCell` remains confined to the instantiator threading (struct fields and
   signatures, poly.rs:512–:996), `Ordering` sits in one region; only `GenericTypes`
   is genuinely spread (48 sites). No forced circularity: the mutual recursion
   `poly_call_term → poly_eliminator_call → poly_walk → poly_call_term` (crossing
   `poly_destructure_generic`) would be *cut* by a responsibility-shaped split, not
   forced by one.

4. **Both rejected splits stay rejected — re-proposing either is out of bounds.**
   `poly/diagnostics.rs` is the layer-shaped split CLAUDE.md names as wrong, with no
   precedent in this checker (`check.rs` interleaves its 41 error formatters with
   their checks; `declarations.rs` 28, `terms.rs` 12). `poly/eliminator.rs` moves the
   recursive cluster across a file boundary, buying more coupling than lines.

5. **The debt history is one-directional.** First recorded at P7.S12 (3/5, both
   candidates rejected); re-confirmed unchanged at P7.S6c's exit, P7.S11's exit,
   P7b.S8's, S8b's and S8c's growth re-checks, and S6d-PREREQ's Phase-3 re-check
   (260909: "poly.rs remains the known deferred 3-of-5-signals case"). No recorded
   exit has moved the count, and none landed a split.

6. **One seam the file's own layout suggests — a probe candidate, not a
   pre-commitment.** Unification/substitution (`unify_poly_input`, `apply_subst`,
   the `Subst`/`CrossGround` machinery around poly.rs:8222–:11278) is called by the
   walk but does not recurse back into it — a candidate leaf seam (e.g.
   `poly/unify.rs`). Whether cutting there actually reduces the signal count on both
   sides is exactly what the audit must measure. Other candidate seams (the
   instantiation cluster, the signature-grounding region) get the same measurement;
   the spec pre-commits the decision rule, not the seam.

7. **Green is the standing gate** — `cargo fmt --check && cargo clippy -- -D
   warnings && cargo test` (3,429 tests at S6d-PREREQ) — plus the P7/P7b golden
   suites (`tests/phase7_*.rs`, `tests/phase7b_*.rs`). A split that leaves any
   resulting file at 2+ firing signals is churn, not a split: "passes the signal
   re-check" means the cut reduces the count on *both* sides. Unit tests travel with
   the code they cover (the beside-the-stage convention), so a split relocates tests
   and touches no assertion.

## The decisions the spec must structure

- **D1 — split or holdout.** A genuine user ruling, taken at a gate after the audit,
  on the audit's measured dossier. The spec pre-commits what evidence selects which
  branch; the outcome is not pre-committed.
- **D2 — if split: the seam.** Chosen from the audit's measured candidates; confirmed
  at the same gate.
- **D3 — if holdout: the ruling's shape.** Recorded in `docs/roadmap/P7-language-prereqs.md`
  beside the S12/S11 growth records (and reflected in the ROADMAP row): the file
  stays whole, the coupling facts that make every probed cut cost more than it buys,
  and what would have to change to reopen the question. This closes the ticket's
  exit criterion either way.

## Out of bounds

- Re-proposing `poly/diagnostics.rs` or `poly/eliminator.rs` in either shape.
- Any behavior change: the work is structural or documentary; diagnostics text and
  goldens must not move.
- Splitting `parser.rs` (15,944 lines, same standing size fact, no ticket).
- Standing probe conventions apply to the audit: disposable no-commit probes,
  reverted via `git checkout`, on a clean tree.
