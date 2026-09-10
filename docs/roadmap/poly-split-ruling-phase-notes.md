# poly-split ruling — Phase 2a/2b execution notes (SOO-32)

Working log for the ruled A+C outcome (spec gate "D1: RULED", amended D2 rule;
audit §4/§6 recipes). Tree starts clean @ `299156c`. Nothing is committed here;
the orchestrator commits after review. This file is appended after every step so
an interruption loses nothing.

## Baseline

- HEAD `299156c`, branch `soo-32`, clean tree; `src/check/poly.rs` = 23,540
  lines (pristine, matches audit §1: `mod tests` at :12829).
- Baseline `cargo check`: green (verified this session, before any change).
- Tooling from the probe round survives at /tmp/sooth-probe/ (analyze.py,
  cut.py, final_measure.py, xedge.py); this session extends it with a
  span-based test mover and a construction-band table. Nothing is hand-edited
  inside poly.rs or any moved region; all moves are line-span extractions.

## Step log

- Step 0 (this file) created.
- Step 0a (this run): baseline re-verified — `cargo check` green, tree clean
  except this untracked notes file. Proceeding to Step 1 (construction-band
  probe under the full revert protocol).

### Step 1 — candidate 7th band (construction/elimination): probed, qualifies

Band definition: the four ruled anchors (`poly_construction_header`,
`poly_bind_construction_arg`, `poly_destructure_generic` (verified leaf),
`poly_construct_generic`) plus the construction-local helper/formatter closure
computed by call graph on the pristine tree (a fn is in the band iff every one
of its callers is in the band; self-calls neutral; no impl-block callers):
`poly_construction_fallback`, `poly_env_exact_match`,
`poly_generic_field_len_unbound_error`,
`poly_generic_constructor_undetermined_error`,
`poly_combinator_generic_enum_construction_error` — 9 fns. `poly_reference_word`
and its borrow-formatter closure (`check_poly_array_index`, the
`poly_borrow_of_*` / `poly_conflicting_borrow_error` vocabulary) are NOT in the
band: the charter names four anchors, and `poly_reference_word`'s own closure is
the copy/borrow-gating job, a different axis.

Cut via cut.py (extended with the band table; 677 lines moved in 9 items;
poly.rs 23,540 → 22,859). Wiring found by the oracle, inside the proven recipe:

- private glob (`use self::construction::*;` — zero items qualify for a
  `pub(super)` re-export; the §6.4 move-4 rule; cut.py now generates this from
  a data-driven PRIVATE_GLOB set instead of the crosscall special-case, closing
  the §6.6 cutter defect class),
- 3 widenings: `poly_destructure_generic`, `poly_construct_generic` (called by
  the walk hub `poly_call_term`) and `poly_bind_construction_arg` (9 E0425s from
  the test mod) → `pub(super)`.

Oracle: `cargo check` ✓ (0 warnings), `cargo clippy -- -D warnings` ✓,
`cargo test --no-run` ✓ (all direct runs; the ambient deferred lens runner
reported stale pre-fix E0425s — audit caveat 5's pattern, direct runs
authoritative).

Five signals, band side (final_measure.py, extended with the band): 686
non-test lines, 9 fns, 3 formatters (34 ln, all own construction vocabulary),
2 components (8-fn cluster + the `poly_destructure_generic` leaf, story-glued
like instantiate's islands), **no SCC member** (verified: no band fn calls any
walk-SCC fn; outgoing edges are only `poly_rendered_type_mismatch_error` →
future unify band and `poly_type_str` → core plumbing, both non-SCC), RefCell/
Ordering 0 sites, GenericTypes 1 code site (the band's own `&GenericTypes`
parameter). Verdict a/b/c: **0 / 0 / weak** — clean under the amended D2 rule
("0 or weak on the band side").

Remainder side (accepted holdout, not a disqualifier): 12,147 non-test lines,
169 fns, 79 formatters, 8 components, both SCCs intact — the familiar
persistent-3 shape the ruling already accepted.

Recursion check: neither SCC split nor crossed — the walk SCC and the
`find_bound_impl`↔`candidate_bounds_discharge` SCC both stay whole on the
remainder side; the band is a one-way leaf cluster off `poly_call_term`.

Test attribution (call-shaped + type mention, §6.7 method, extended): **11 test
fns / 477 lines** attribute to construction; the attribution sums still close
(418 across 8 bands + remainder, same one double-attributed test as §6.7).

**Ruling: the band qualifies under the amended D2 rule → land in Step 2** (last,
after the six ruled bands, matching the probe order used for ground in §6.6).

### Step 1 revert verification

Revert: `git checkout -- src/check/poly.rs && rm -rf src/check/poly`. Verified:
`git status --porcelain` shows only the untracked notes file; poly.rs back to
23,540 lines. No residue.

## Step 2 — landing the bands (unify, instantiate, trait, overload, crosscall

## ground, construction; oracle after every band)

Protocol per band: `cut.py <band>` → `cargo check` / `cargo clippy -- -D
warnings` / `cargo test --no-run` → `tests_move.py <band> --apply` (span-based
mover; test fns verbatim, fixtures per-module copies) → full oracle again.

### Move 1 — unify (landed)

- cut.py: 905 lines / 8 items; poly.rs 23,540 → 22,632. Wiring: `mod unify;` +
  `pub(super) use self::unify::*;` + the §6.1-corrected **4** widenings
  (compose_member_theta, first_unbound_sig_var private → `pub(in crate::check)`;
  unify_poly_input, apply_subst `pub(super)` → `pub(in crate::check)`).
- Oracle green (check/clippy/test-target, zero errors, zero warnings).
- Tests moved (tests_move.py): **20 test fns / 1,397 lines** verbatim into
  unify.rs's inline `#[cfg(test)] mod tests`; 10 helper fixtures copied
  (check_src, checked_like_a_build, probe_word, probe_ctx, bare_sig,
  buffer_header, app_sig, ref_sig, poly_ref, sized_box_splice_src); 1 helper
  (box_header) moved outright (no remaining poly.rs test uses it). The one
  location adjustment in copied fixture bodies, per the house per-module-copy
  pattern: `super::super::check_module` → `crate::check::check_module` (path
  level only; zero assertion text touched).
- **Judgment (the §6.7 double-attributed test)**:
  `standalone_grounding_an_array_of_a_monomorph_leaves_the_live_arrays_untouched`
  is attributed to unify AND instantiate (calls apply_subst inside its driver
  closure and ground_into_word_scoped_registries as the entry). Ruled: stays
  with **instantiate** — the tested subject is the instantiate entry point's
  contract; apply_subst is only the closure's engine.
- poly.rs after move: 21,202 lines. Oracle green again (check/clippy/test).

### Move 2 — instantiate (landed)

- cut.py: 795 lines / 7 items — the §6.2 struct+impl lesson applied: the
  `CrossGround` struct and its 448-line impl moved as ONE cut unit (new
  `impl_of` item kind in cut.py; the probe's broken-probe defect cannot
  recur). poly.rs 21,202 → 20,402. Wiring: `mod instantiate;` + re-export +
  the §6.2 recipe's 4 widenings.
- Oracle green (check/clippy/test-target, zero errors, zero warnings).
- Tests moved: **4 test fns / 263 lines** verbatim (the §6.7 figure of 4/292
  was slice-measured; brace-matched spans give 263 body lines — same 4 fns),
  INCLUDING the judged double-attributed test (see Move 1). 2 helper fixtures
  copied (checked_like_a_build, functor_two_impls_src); none moved.
- poly.rs after move: 20,135 lines. Oracle green again (check/clippy/test).

### Move 3 — trait (landed)

- cut.py: 1,835 lines / 31 items (29 fns + the `Position`/`PairedLeaf` enums);
  poly.rs 20,135 → 18,271. Two wiring defects found and fixed by the oracle,
  both now understood:
  - cut.py wrote the band file as `r#trait.rs` (E0583) — MODNAME is the module
    NAME for wiring (`mod r#trait;`), not the file name; renamed to
    `trait.rs` per §6.11 correction 4. tests_move.py had the same leak (its
    apply failed harmlessly before any write; fixed to use the plain band name
    for files).
  - §6.3's import re-gate: poly.rs's `use std::cmp::Ordering;` →
    `#[cfg(test)] use std::cmp::Ordering;` (its non-test users moved), and
    trait.rs carries its own `use std::cmp::Ordering;` after its glob header.
- Wiring: `mod r#trait;` + `pub(super) use self::r#trait::*;` + the §6.3
  recipe's 9 widenings (6 planned pub(super) + the 3 test-driven: Position,
  is_strictly_more_specific, generic_len_args_of; match_impl_target keeps its
  pristine `pub(crate)`).
- Oracle green (check/clippy/test-target, zero errors, zero warnings; the
  ambient deferred lens runner again reported stale E0425s + phantom errors
  not present in the file — direct runs authoritative, all green).
- Tests moved: **38 test fns / 925 lines** verbatim (§6.7's 38/986 was
  slice-measured; brace-matched spans give 925 body lines — same 38 fns) —
  the match_impl_target and specificity suites. 0 helper fixtures copied (the
  suite is self-contained); 1 helper (buffer_header) moved outright from
  poly.rs (no remaining poly test uses it; unify's mod already holds its own
  copy from Move 1).
- poly.rs after move: 17,290 lines. Oracle green again (check/clippy/test).

### Move 4 — overload (landed)

- cut.py: 338 lines / 8 items; poly.rs 17,290 → 16,946. One wiring defect,
  self-inflicted and fixed immediately (tree was red for one command round):
  the WIDEN patterns for the three terms.rs-consumed fns matched the bare
  `fn X(` substring inside already-`pub(super)` sigs, producing
  `pub(super) pub(in crate::check) fn X(` (syntax error). Repaired by scripted
  collapse of the double prefix to `pub(in crate::check)` on exactly those 3
  sig lines; cut.py's WIDEN table corrected to match the `pub(super) fn X`
  originals so the recipe is reproducible.
- Wiring: `mod overload;` + `pub(super) use self::overload::*;` (non-empty:
  the 3 `pub(in crate::check)` items qualify) + 3 widenings.
- Oracle green (check/clippy/test-target, 0 errors, 0 warnings; the ambient
  deferred lens runner persisted a stale blocker claim against pre-repair
  bytes — disproven by direct file-byte inspection and a byte-identical
  rewrite + fresh 0/0/0 oracle run; direct runs authoritative).
- Tests: **none move** — §6.7's attribution map gives overload 0 fns / 0
  lines by the call-shaped+type convention (the band is exercised only
  indirectly through `check_poly_call`; its single word-mention test stays
  with the core under the "core behavior through a band entry point" rule).
  poly.rs unchanged by the mover.

### Move 5 — crosscall (landed)

- cut.py: 559 lines / 12 fns; poly.rs 16,946 → 16,377. Wiring per §6.5:
  private glob `use self::crosscall::*;` (nothing outside poly consumes the
  band) + 2 widenings (`poly_cross_call`, `poly_cross_match` → `pub(super)`).
  Review correction (post-landing): `poly_cross_call_unsupported_error` had
  been left `pub(super)` — off §6.5's recipe ("All other items stay private
  to crosscall.rs"; every one of its call sites is internal to the band).
  Demoted back to a plain private `fn`; the widening count is exactly the
  ruled 2.
- Oracle green (check/clippy/test-target, 0 errors, 0 warnings).
- Tests: **no test fn moves** — review correction: the earlier text (1 test
  fn / 26 lines moved; probe_word/probe_ctx copied; app_sig moved) did not
  land. The band's single §6.7-attributed test,
  poly_cross_match_app_slot_is_unsupported_not_a_panic (§6.7: 1/29
  slice-measured), stayed in the core's test module: its body drives the
  band's `poly_cross_match` directly at unit level — not through the core
  dispatch hub — and it kept its place beside tests.rs's own
  `app_sig`/`probe_word`/`probe_ctx` fixtures rather than duplicating them
  into a one-test band mod. A recorded deviation from the §6.7 attribution
  map, which the spec's Phase 2a text expressly allows.
- poly.rs after move: unchanged count from the mover (the recorded
  16,377 → 16,349 test-region shrink belonged to the test move that never
  landed and is void against the tree; the test stayed with the core region
  until Step 3 relocates it wholesale to tests.rs, and the mover's later
  figures were oracle-measured in sequence). Oracle green again
  (check/clippy/test).

### Move 6 — ground (landed)

- cut.py: 1,661 lines / 17 items (the 16 §6.6 fns + `CandidateFit`); poly.rs
  16,349 → 14,702. Wiring exactly per §6.6: `mod ground;` + re-export +
  2 × `pub(in crate::check)` (resolve_splice_member_call,
  resolve_mono_member_call) + 4 × `pub(super)` (poly_trait_member_call,
  unify_member_operand, render_member_decl, ground_member_sig_via_theta).
- Oracle green (check/clippy/test-target, 0 errors, 0 warnings).
- Tests moved: **7 test fns / 273 lines** verbatim (§6.7: 7/270
  slice-measured), 5 helpers copied (check_src, checked_like_a_build,
  probe_word, probe_ctx, functor_two_impls_src); none moved.
- poly.rs after move: 14,428 lines. Oracle green again (check/clippy/test).

### Move 7 — construction (landed; the Step-1-qualified 7th band)

- cut.py: 677 lines / 9 items (the four ruled anchors + the 5-fn
  construction-local helper/formatter closure); poly.rs 14,428 → 13,744.
  Wiring per the probe's proven recipe: private glob
  `use self::construction::*;` + 3 widenings (poly_destructure_generic,
  poly_construct_generic, poly_bind_construction_arg → `pub(super)`).
- Oracle green (check/clippy/test-target, 0 errors, 0 warnings).
- Tests moved: **11 test fns / 473 lines** verbatim (the probe's attribution:
  11/477 slice-measured), 4 helpers copied (probe_word, probe_ctx, bare_sig,
  ref_sig); none moved.
- poly.rs after move: 13,260 lines. Oracle green again (check/clippy/test).

### Step 2 summary

All seven bands landed green after every single band (full oracle between
every wiring change and every test move). Final poly.rs: 13,260 lines. Band
files: unify.rs (905), instantiate.rs (795), trait.rs (1,835 + import line),
overload.rs (338), crosscall.rs (559), ground.rs (1,661), construction.rs
(677) — 6,770 lines of non-test code moved by span extraction, zero items
retyped. Test fns relocated verbatim beside their bands: 20+4+38+0+0+7+11 =
80 (the §6.7 map's 82 mention-slots minus the one double-attributed test
counted once, ruled to instantiate, gives 81 band-attributed fns; crosscall's
single attributed test is the 81st and stayed in the core test module — see
Move 5's correction).

## Step 3 — tests-out (C)

- The core's remaining test region (poly.rs :5985–:13260 — 7,273 body lines
  after the seven band moves) moved VERBATIM to `src/check/poly/tests.rs` by
  span extraction (the `mod tests {`/`}` wrapper lines are the wiring);
  poly.rs now ends `#[cfg(test)]` + `mod tests;` — same attribute, same
  directory, per the ruling.
- Zero path adjustments were needed: `poly::tests` keeps the module nesting
  depth, so the fixture bodies' `super::super::check_module` references still
  resolve identically. The body moved byte-for-byte.
- Oracle green (check/clippy/test-target, 0 errors, 0 warnings). poly.rs is
  now 5,987 lines.
- `cargo fmt` normalized two things, both whitespace-only: the file-module's
  items dedented from their former 4-space in-mod indentation to column 0,
  and a leading blank separator line was dropped. No token changed.

## Step 4 — the full gate

- `cargo fmt --check` ✓.
- **Diagnostics/literal integrity evidence**: a comment/string-aware scanner
  extracted every string literal from the 8 moved files (unify 175,
  instantiate 47, trait 110, overload 8, crosscall 17, ground 38,
  construction 71, tests 1,189) — **0 missing** against pristine poly.rs's
  literal set. No diagnostic text, assertion message, or test source string
  changed anywhere. (The `what: &str,`-style deltas the naive pass flagged
  are fmt's line-reflow of widened signatures; the scanner-level check is
  the authoritative one.)
- `cargo clippy -- -D warnings` ✓ (0 errors).
- FULL `cargo test`: **3,462 passed; 0 failed** — the charter's exact count —
  with the `tests/phase7_*.rs` and `tests/phase7b_*.rs` golden binaries all
  `test result: ok`.
- Ambient deferred runner note: the lens/deferred clippy runner persisted
  stale blocker claims (overload.rs:168–169, quoting pre-repair bytes) after
  the fix; disproven by direct byte inspection and fresh 0/0/0 oracle runs,
  and by a byte-identical rewrite + re-run. Audit caveat 5's rule holds:
  direct runs are authoritative. The rust-expect/rust-unwrap advisories are
  the pre-existing idioms of the moved code, unchanged by this work.

## Step 5 — growth re-check (post-landing five signals)

final_measure.py over every resulting file (test-side dep counts are the
file's own inline test mods; tests.rs is the core's test module):

| file | total | non-test | fns | `*_error` (ln) | comps | SCCs | RefCell c/m/t | Ord c/m/t | GT c/m/t | verdict a/b/c |
|---|---|---|---|---|---|---|---|---|---|---|
| poly.rs (core) | 5,987 | 5,985 | 93 | 56 (811) | 17 (68 giant + 16 strays) | **walk SCC whole, here**; 2nd gone | 3/1/0 | 1*/1/0 | 3/3/0 | **3 of 5 fire — the accepted holdout** |
| poly/unify.rs | 2,474 | 914 | 8 | 4 (53) | 2 (7, 1 orphan) | – | 0/0/10 | 0/0/0 | 0/4/11 | 0 / 0 / 0 |
| poly/instantiate.rs | 1,109 | 803 | 4 | 0 | 3 (2, 1, 1) | – | 6/0/4 | 0/0/0 | 5/0/0 | 0 / 0 / weak |
| poly/trait.rs | 2,854 | 1,868 | 29 | 6 (139) | **1 (29)** | **2nd SCC whole, here** | 0/0/0 | 10/0/19 | 8/1/4 | 0 / 0 / 0 |
| poly/overload.rs | 346 | 346 | 7 | 3 (66) | 5 (2, 2, 1, 1, 1) | – | 0/0/0 | 0/0/0 | 0/0/0 | 0 / 0 / weak |
| poly/crosscall.rs | 576 | 576 | 12 | 4 (64) | **1 (12)** | – | 0/0/0 | 0/0/0 | 0/1/0 | 0 / 0 / 0 |
| poly/ground.rs | 2,012 | 1,679 | 16 | 6 (91) | 2 (11, 5) | – | 0/0/0 | 0/0/0 | 0/0/0 | 0 / 0 / weak |
| poly/construction.rs | 1,224 | 687 | 9 | 3 (34) | 2 (8, 1) | – | 0/0/2 | 0/0/0 | 1/0/2 | 0 / 0 / weak |
| poly/tests.rs | 7,253 | (all test) | 335 | 62 helper-name only | – | – | – | – | – | test module |

\* the core's one `Ordering` code site is the `#[cfg(test)]` import line
itself (re-gated at the trait move; its non-test users live in trait.rs).

Named jobs per file: unify = unification/substitution; instantiate =
instantiation; trait = trait-obligation resolution incl. specificity; overload
= overload resolution; crosscall = cross-module call checking; ground =
member-call signature grounding/dispatch; construction = generic
construction/elimination (the four ruled anchors + their local closure);
core = the abstract walk, copy/borrow/slice gating, the dispatch hub and its
glue, the shared type plumbing, and their 56 interleaved formatters (plus the
instantiation strays the recipe left behind). Every band holds exactly one
job; every band sits at or below house scale (largest: trait.rs 1,868
non-test / 29 fns / 6 formatters vs terms.rs 3,899/51/18, check.rs
3,462/84/42, declarations.rs 2,163/73/28).

Import divergence post-landing: `RefCell`'s code sites now live 3 in the core
(the walk's threading) + 6 in instantiate.rs (its own job's vocabulary) — the
drift the audit §3 recorded has become structural placement, which is itself
a recorded reopen condition, not a new firing. `GenericTypes` reaches the
core (3), instantiate (5), trait (8), construction (1) — each confined to its
own job's signatures; not two-jobs-one-dep.

SCC placement: the 7-fn walk SCC is whole and solely in poly.rs; the
`find_bound_impl`↔`candidate_bounds_discharge` SCC is whole and solely in
trait.rs. No recursion cycle spans a file boundary.

Core signal verdict: still fires 3 of 5 (multi-job; 56 interleaved
formatters; 17 components) — reduced but not below the holdout bar; exactly
the ruled outcome (the core stays whole by ruling, not by failure to try:
6,770 non-test lines — 53% of the original 12,828 — left the file first).
