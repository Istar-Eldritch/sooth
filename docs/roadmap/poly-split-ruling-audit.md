# poly.rs ruling — Phase 1 measured audit

SOO-32. Phase 1 of [poly-split-ruling-spec](./poly-split-ruling-spec.md): fresh
measurements against **soo-32 @ `575a5a1`**, branch `soo-32`, taken **2026-09-10**.
No committed file changed; every structural experiment was a disposable no-commit
probe on a clean tree, reverted with `git checkout` after measurement. All line
anchors below are from the pristine tree unless a probe's post-cut state is named.
This dossier is evidence for the D1 gate; it makes no ruling.

## 1. Size, split test from non-test

| measure | value | anchor |
|---|---|---|
| `wc -l src/check/poly.rs` | 23,540 | (was 23,535 at the brief's recon @ `9ed2890`) |
| `#[cfg(test)]` attribute | poly.rs:12829 | |
| `mod tests` opens | poly.rs:12830 | unchanged from discovery |
| non-test source | 12,828 lines (1–12828); 12,829 counting the attribute | |
| test region | 10,712 lines (12829–23540), 45.5% of the file | |

The ruling's size argument must use the non-test number: 12.8k lines of stage
code, not 23.5k.

## 2. The three firing signals, re-measured

### (a) Does X and Y and Z — distinct jobs

Measured: **at least 5 distinct jobs** (the brief named 4; the trait-obligation
band is a fifth). Each is anchored to named, top-level functions:

1. **Signature grounding / member-call dispatch** — `member_fits_operands`
   poly.rs:1330, `unify_member_operand` :1399, `render_member_decl` :1500,
   `dispatchable_input_pos` :1537, `substitute_member_var` :1557,
   `resolve_splice_member_call` :1607, `ground_member_sig_via_theta` :2122,
   `resolve_mono_member_call` :2287, `poly_trait_member_call` :2808.
2. **The abstract term walk** — `check_poly_body` :980 → `poly_walk` :1096 →
   `poly_term` :1166 → `poly_call_term` :2983 → `poly_eliminator_call` :4837 →
   `poly_walk_arms` :5360 → `poly_destructure_generic` :6419, plus walk-cycle
   members `poly_combinator_call` :5765 and `poly_ground_quotation_literal` :4579.
3. **Instantiation** — `discover_transitive_instantiations` :8089,
   `intern_composed_bundles` :8208, the `CrossGround` engine :8222/:8238–:8689,
   and the registry threading `WordScopedRegistries` :483,
   `ground_into_word_scoped_registries` :511, `arrow_stand_in` :595.
4. **Unification / substitution** — `unify_poly_input` :10807, `apply_subst`
   :11278, `compose_member_theta` :9475, `first_unbound_sig_var` :9714.
5. **Trait-obligation resolution** — `select_most_specific` :8762,
   `find_bound_impl` :8924, `resolve_user_bound` :9041, `match_impl_target`
   :9881, `specificity` :10688, `is_strictly_more_specific` :10765.

(Cross-module call checking — `poly_cross_call` :3913, `poly_cross_relate`
:4054, `poly_cross_match` :4091, `poly_cross_output` :4268 — and overload
resolution — `resolve_poly_overload` :7205, `poly_sig_unifies` :7287,
`resolve_combinator_overload` :7425, `poly_sig_could_match` :7336 — are further
distinguishable jobs. **Signal fires.**)

### (b) High- and low-level mixed — `*_error` formatters

Measured: **82 top-level `fn *_error` definitions in non-test code** (also 82 at
any indent — all are top-level), first `splice_member_unbound_var_error`
poly.rs:2205, last ending :12740, **1,877 lines** of formatter bodies. Another
70 `fn *_error` helpers live in `mod tests` (152 total; the brief's "155" was
not reproducible under either counting rule — see caveats).

Interleaving with walk logic is structural, not incidental: 4 of the 6 walk
anchors are defined *after* the first formatter (`poly_call_term` :2983,
`poly_eliminator_call` :4837, `poly_walk_arms` :5360, `poly_destructure_generic`
:6419); the `mono_member_*` error block :2683–:2807 sits immediately before
`poly_call_term` :2983; and the late formatter band :11198–:12740 is interleaved
between `apply_subst` :11278 and `poly_type_str` :12751. **Signal fires.**

### (c) Functions that never call each other — top-level clusters

Method: call graph over the 178 top-level functions of poly.rs; an edge `f → g`
exists when `g`'s name occurs call-shaped (``g(``, not ``::g(``) inside `f`'s
body span. "Into the walk cluster" = transitive reachability of any of the six
walk anchors (§2a item 2).

- **169 of 178 top-level functions have no transitive call edge into the walk
  cluster.** Only 3 non-walk functions call into it at all: `poly_term` :1166,
  `poly_combinator_call` :5765, `poly_ground_quotation_literal` :4579.
- Undirected components: 8 — one 170-fn giant plus 7 strays: the
  `{discover_transitive_instantiations, intern_composed_bundles}` pair
  (:8089–:8221; driven from check.rs:1138, not from poly.rs),
  `poly_mentions_len_var` :4382, `no_combinator_overload_matches_error` :7477,
  `enqueue_new` :8690, `body_calls_a_poly_word` :8709,
  `overloaded_cross_call_error` :8721, `inline_callee_cross_call_error` :8741.
- Named clusters on the no-edge-into-walk side: the 82-formatter vocabulary
  (:2205–:12740), the member-call band (:1330–:2982), the cross-call band
  (:3913–:4533), the trait-obligation band (:8762–:9474, :9881–:10806), the
  unification pair (:10807, :11278), the registry threading (:483–:680), the
  instantiation core (:8089–:8689).

(The 7 "strays" are reachable from `CrossGround`'s *impl methods*, which are not
nodes in a top-level-only graph — see caveats. The one-directional finding
stands regardless.) **Signal fires.**

## 3. The two quiet signals, re-measured

### Import divergence

`grep` site counts and regions (comment-only sites noted):

- **`RefCell`: 27 sites** — import :1; threading signatures :512, :520, :529
  (`ground_into_word_scoped_registries`), :599 (`arrow_stand_in`), :694
  (`check_poly_combinator_standalone`), :996 (`check_poly_body`); comment-only
  mention :5073 (inside `poly_eliminator_call`); **drift beyond the recorded
  :512–:996 band: :8098 (`discover_transitive_instantiations` signature) and
  :8235 (`CrossGround` field)**; 17 further sites in tests from :14667.
- **`Ordering`: 31 sites** — import :2; comment-only :3343 (inside
  `poly_call_term`); code sites :8787 (`select_most_specific`) and :10698/:10759
  (`specificity`/`bound_set_tiebreak` band); 19 in tests (:21552–:23437). One
  job (specificity ranking), two physical regions.
- **`GenericTypes`: 48 sites** (matches the brief) — 26 non-test spread over ~7
  regions touching **every** job: import :4; registry band :512–:996 (5);
  `poly_cross_call` :4037; `poly_eliminator_call` :5080, :5081, :5129;
  `poly_construction_header` :6065; `discover_transitive_instantiations` :8098;
  `CrossGround` :8235; `select_most_specific` :8768; the trait/matching band
  :9877–:10697 (7); the unification band :11195, :11308, :11446, :11448; 22 in
  tests.

**Verdict: does not fire as a binary signal yet, but is no longer static.**
`RefCell` has drifted out of its recorded band into the instantiation cluster,
and `GenericTypes` now reaches every job cluster in the file. Both drifts are
exactly along the seams probed in §4. What would make it fire: a dependency
becoming load-bearing for two jobs that a split would want to separate
(e.g. `GenericTypes` threading through the walk proper).

### Forced circularity

The recursion cycle, edge by edge (call-site anchors):

```
poly_call_term :2983 --:3557--> poly_eliminator_call :4837
                           --:5250--> poly_walk_arms :5360
                           --:5427--> poly_walk :1096
                           --:1142--> poly_term :1166
                           --:1259--> poly_call_term   (cycle closes)
```

plus a direct shortcut `poly_call_term` :3459 → `poly_walk`, and a third entry
`poly_call_term` :3512 → `poly_combinator_call` :5765 → `poly_walk_arms`.

Correction to the brief: **`poly_destructure_generic` is not a cycle member.**
It is called from `poly_call_term` :3647 and has no outgoing call to any
top-level poly.rs function — it hangs off the cycle hub as a leaf. (A second,
pre-existing mutual-recursion pair exists: `find_bound_impl` :8924 ↔
`candidate_bounds_discharge` :8856, in the trait band.)

**Verdict: does not fire as an obstacle for the probed seams** — none of them
contains a cycle member, so each cut would leave the cycle whole on the walk
side rather than being forced across a file boundary. It *would* be crossed by
any cut that separates `poly_call_term` from the walk fns (the rejected
eliminator shape, out of bounds here).

## 4. Candidate seams, probed

Protocol for each seam: physically move the cluster into
`src/check/poly/<name>.rs` (poly.rs keeps `mod <name>;` + re-export; edition
2021 permits `poly.rs` + `poly/`), adjust module wiring until the crate
compiles, run `cargo check` + `cargo clippy -- -D warnings` + compile the test
target, measure the firing signals on both sides, then fully revert
(`git checkout -- src/check/poly.rs; rm -rf src/check/poly`) and verify a clean
tree before the next probe. Zero behavior changes; the only edits are module
wiring (visibility widenings and re-exports). Tests were not physically
relocated; the test-line split is attributed per the spec ("count tests as
belonging to whichever side holds the code they cover"), by test-body mention of
moved functions.

Shared wiring fact discovered by every probe: sibling checker modules
(`combinators.rs`, `terms.rs`) reach `poly.rs`'s `pub(super)` items through
check.rs's glob re-export chain, and a glob re-export does **not** carry a
module's private glob-imports. Moved items therefore need `pub(in crate::check)`
(kept-visible) plus a `pub(super) use self::<name>::*;` re-export in poly.rs to
preserve the pre-move visibility surface exactly. This is cheap (a few lines)
but it is coupling that every split must pay.

### Probe 1 — unification/substitution → `src/check/poly/unify.rs`

Moved (913 lines, 8 fns): `compose_member_theta` :9475, `first_unbound_sig_var`
:9714, `unify_poly_input` :10807, its four error formatters
`poly_generic_not_yet_groundable_error` :11198,
`poly_app_len_domain_unsupported_error` :11218,
`poly_constructed_reference_error` :11245,
`poly_rendered_type_mismatch_error` :11260, `apply_subst` :11278–:11599.
Note: `Subst` itself lives in ast.rs:2912 and moves nowhere; and `CrossGround`
was **reassigned to the instantiation cluster** (probe 2) on measured evidence:
it is constructed only at poly.rs:8131, inside
`discover_transitive_instantiations` — the brief's "Subst/CrossGround
machinery" grouping does not match the construction graph.

- Compile: `cargo check` ✓, `cargo clippy -- -D warnings` ✓, test target ✓.
  Wiring: `mod unify;` + `pub(super) use self::unify::*;` + 2 visibility
  widenings (`compose_member_theta`, `first_unbound_sig_var`: private →
  `pub(in crate::check)`); one intermediate fix round (sibling-visibility chain
  above).
- Seam side: 918 lines; 1 job; 4 formatters, all the cluster's own vocabulary
  (80 lines); 2 components (7-fn cluster + 1 orphan —
  `poly_constructed_reference_error` is called only from the remainder: a
  seam-precision miss, not a structural cluster); no SCC; 0 `RefCell`/`Ordering`
  sites; `GenericTypes` in 4 doc comments only.
- Remainder side: 22,629 lines (`mod tests` at :11919); 170 top-level fns; 78
  formatters, first :2207; 16 undirected components; **both recursion SCCs
  intact**; ≥4 jobs remain; `RefCell` 26 non-comment sites.
- Tests: 21 test fns / 1,506 lines attributed to the seam; 396 fns / 9,205
  lines remain in poly.rs.
- Cross-seam edges: seam → remainder 11 targets, **all leaf formatters/renderers
  or `fence_quotation_literal_slot` :9540 — none reaches the walk SCC**
  (verified transitively). Remainder → seam: 15 callers, incl.
  `check_poly_call` :7516, `ground_member_sig_via_theta` :2122,
  `poly_sig_unifies` :7287, `poly_sig_could_match` :7336, `resolve_user_bound`
  :9041, `poly_call_term` :2983 (via `poly_rendered_type_mismatch_error`);
  outside poly.rs, `combinators.rs` calls `apply_subst`/`unify_poly_input`
  through the re-export chain.
- Cycle effect: **disjoint** — neither crossed nor reduced; the walk cycle stays
  whole inside poly.rs.

### Probe 2 — instantiation → `src/check/poly/instantiate.rs`

Moved (802 lines): `WordScopedRegistries` :483, `ground_into_word_scoped_registries`
:511, `arrow_stand_in` :595 (chunk :478–:653), and
`discover_transitive_instantiations` :8089, `intern_composed_bundles` :8208,
`CrossGround` struct + impl :8222–:8689 (chunk :8061–:8686) — 4 top-level fns
plus the ~468-line `CrossGround` struct/impl machinery.

- Compile: ✓ / ✓ / ✓ after one fix round. Wiring: `mod instantiate;` + re-export
  - 4 widenings (3 × `pub(super)` → `pub(in crate::check)` for
  `ground_into_word_scoped_registries`, `discover_transitive_instantiations`,
  `WordScopedRegistries` — its 6 fields are already `pub` :484–:490; 1 ×
  private → `pub(super)` for `arrow_stand_in`).
- Seam side: 806 lines; 1 job; 0 formatters; **3 undirected components** —
  `ground_into_word_scoped_registries` and `arrow_stand_in` are mutually
  disconnected islands, and neither calls `discover_transitive_instantiations`;
  the three are glued only by shared types and the single instantiation story;
  no SCC; `RefCell` 6 + `GenericTypes` 5 code sites, both confined to this one
  job (not divergence).
- Remainder side: 22,740 lines (`mod tests` at :12030); 174 fns; 82 formatters;
  7 components; **both SCCs intact**; `RefCell` 20 non-comment sites (the
  signature threading of `check_poly_combinator_standalone` :518ff and
  `check_poly_body` :820ff stays with the walk side).
- Tests: 7 fns / 402 lines attributed to the seam; 410 fns / 10,309 lines
  remain.
- Cross-seam edges: seam → remainder 6 (`apply_subst`, `resolve_user_bound`,
  `body_calls_a_poly_word` :8709, `enqueue_new` :8690,
  `overloaded_cross_call_error` :8721, `inline_callee_cross_call_error` :8741);
  remainder → seam: `check_poly_combinator_standalone` :681 →
  `ground_into_word_scoped_registries`/`arrow_stand_in`; and
  `discover_transitive_instantiations` is entered from **check.rs:1138**, so the
  seam is not even private to the poly.rs/instantiate.rs pair.
- Cycle effect: disjoint, but the seam is not a leaf — it calls six remainder
  functions, including into the trait band. No transitive file cycle forms.

### Probe 3 — signature-grounding region → `src/check/poly/ground.rs`

Moved (1,677 lines, 16 fns): the member-call band, original poly.rs
:1330–:2982 — `member_fits_operands` :1330, `candidate_fitting_the_operands`
:1338, `enum CandidateFit` :1367, `unify_member_operand` :1399,
`render_member_decl` :1500, `dispatchable_input_pos` :1537,
`substitute_member_var` :1557, `resolve_splice_member_call` :1607,
`ground_member_sig_via_theta` :2122, `splice_member_unbound_var_error` :2205,
`splice_member_ctor_image_error` :2237, `resolve_mono_member_call` :2287,
`mono_member_no_dispatch_error` :2683,
`mono_nullary_member_no_instantiation_error` :2705,
`mono_member_unroutable_error` :2745, `mono_ambiguous_member_error` :2768,
`poly_trait_member_call` :2808. (Operationalization of "the signature-grounding
region": this band grounds member signatures against operands —
`ground_member_sig_via_theta`, `unify_member_operand`, `render_member_decl` —
and its entry points are called from terms.rs:929/:989.)

- Compile: ✓ / ✓ / ✓ after **three fix rounds — the test target needed the most
  wiring of any probe**: `mod ground;` + re-export + 6 widenings
  (`resolve_splice_member_call`, `resolve_mono_member_call` →
  `pub(in crate::check)`; `poly_trait_member_call`, `unify_member_operand`,
  `render_member_decl`, `ground_member_sig_via_theta` → `pub(super)`), because
  the test mod calls the last three directly (9 + 1 E0425s in `mod tests`
  before the widenings).
- Seam side: 1,680 lines; 1 job; 6 formatters (207 lines), all own vocabulary;
  2 components (11-fn splice cluster + 5-fn mono-dispatch island); no SCC; 0
  `RefCell`/`Ordering`/`GenericTypes` sites.
- Remainder side: 21,865 lines; 162 fns; 76 formatters, first :2754; 9
  components — including a **new orphan trio**
  (`ambiguous_trait_member_error` :11799, `no_candidate_fits_operands_error`
  :11832, `joined_with_and` :11869, called only by the moved
  `poly_trait_member_call`); **both SCCs intact**.
- Tests: 24 fns / 711 lines attributed to the seam; 393 fns / 10,000 lines
  remain.
- Cross-seam edges — the finding that disqualifies this seam: seam → remainder
  12 targets **including `check_poly_call` :7516 (which drives the walk SCC)
  and `find_bound_impl` :8924 (a member of the second SCC)** — the seam
  transitively reaches *both* recursion cycles — while the remainder calls into
  the seam (`poly_call_term` → `poly_trait_member_call`; `check_poly_call` →
  `dispatchable_input_pos` :1537). Cross-file mutual dependency touching the
  recursion machinery.
- Cycle effect: the 7-fn walk SCC itself stays whole on the remainder side, but
  the cut **crosses** coupling to it (bidirectional references, seam reaches
  both SCCs). Worst of the three probes.

## 5. Per-seam verdict (evidence for the D1 gate — not the ruling)

Rule under test (spec, D1/D2): a seam qualifies only if **both sides < 2 firing
signals** post-cut **and** the cut reduces rather than crosses the recursion
cycle.

| seam | post-cut seam side (a/b/c) | post-cut remainder (a/b/c) | cycle | qualifies for D2 |
|---|---|---|---|---|
| unify.rs (unification/substitution, 918 ln) | 0 / 0 / 0 (1 orphan caveat) | 1 / 1 / 1 → **3 firing** | disjoint; not crossed, not reduced | **No** — remainder still 3 |
| instantiate.rs (instantiation, 806 ln) | 0 / 0 / weak (3 mutually disconnected groups) | 1 / 1 / 1 → **3 firing** | disjoint; seam not a leaf (6 back-calls) | **No** — remainder still 3 |
| ground.rs (signature grounding, 1,680 ln) | 0 / 0 / weak (2 islands) | 1 / 1 / 1 → **3 firing** | seam transitively reaches **both** SCCs; bidirectional refs | **No** — remainder 3 **and** cycle coupling crossed |

The load-bearing fact: **every probed cut leaves the remainder at 3 firing
signals.** The remainder keeps the walk SCC, ~76–78 interleaved formatters, and
a multi-job mass no matter which leaf is shaved off; each seam side is clean
(≈0–1 weak signals) but the cut never brings the *other* side under 2. The
three moves together total 3,404 lines (14% of the file); the remainder would
still be ≈9.4k non-test lines and just as multi-job. No seam qualifies under
the pre-committed rule. Whether that means holdout (D3) or a different seam
shape is the user's gate to rule on; this dossier only records that the three
spec-named candidates were measured and none qualifies.

## Measurement caveats

1. **Call graph is approximate.** Edges are name-mentions shaped like calls
   (`name(`) inside top-level function body spans; impl-block methods
   (e.g. `CrossGround`'s ~460-line impl :8238–:8689) are not graph nodes, so
   functions called only from methods (`enqueue_new` :8690,
   `body_calls_a_poly_word` :8709, the cross-call error pair :8721/:8741) appear
   as disconnected strays. Directional findings (§2c, §4 cross-edges) were
   spot-checked against source and hold.
2. **The brief's "155 `*_error` formatters" is not reproducible**: measured
   82 top-level non-test + 70 test-side = 152 by definition count; 476 total
   `*_error` name occurrences. The brief may have used a different pattern;
   §2b states the method used here.
3. **Corrections to the brief recorded in §3**: `poly_destructure_generic` is a
   leaf off `poly_call_term` :3647, not a cycle member; `RefCell` has drifted
   past :512–:996 to :8098 and :8235; `CrossGround` belongs to the
   instantiation cluster by construction graph, not the unification cluster.
4. **Test attribution is by mention**, not physical relocation (the probes left
   `mod tests` in poly.rs, per the spec's probe scope); a real split relocates
   the attributed test fns beside their code.
5. **Compile gates were run directly per probe** (`cargo check`,
   `cargo clippy -- -D warnings`, `cargo test --no-run`): all green on each
   probe's final wiring. An ambient deferred clippy runner reported failures
   during the session; direct runs are authoritative and passed. All remaining
   `expect`/`unwrap` warnings are pre-existing idioms of the moved code — no
   behavior changed.
6. **Probes were sequential on a verified-clean tree**; after each,
   `git checkout -- src/check/poly.rs && rm -rf src/check/poly` restored HEAD
   exactly (`git status` clean, file 23,540 lines, HEAD `575a5a1` re-verified).
   The only intended artifact of Phase 1 is this untracked dossier.

## 6. Job-partition probe (user-directed follow-up)

Follow-up probe round, user-directed after the §5 verdict. Hypothesis under test:
the D1 failure ("remainder stays at 3 firing signals") was measured against
single cuts from an unchanged remainder. A simultaneous partition along job
boundaries may bring every resulting file under the qualifying bar (both sides
< 2 firing signals, cut reduces rather than crosses the recursion cycle).

Protocol: the five job bands (§2a's jobs, minus the walk itself) are moved
**in this order** — `poly/unify.rs`, `poly/instantiate.rs`, `poly/trait.rs`,
`poly/overload.rs`, `poly/crosscall.rs` — keeping the crate compiling after
each move (`cargo check`; `cargo clippy -- -D warnings` + `cargo test --no-run`
when a move settles). Wiring follows the §4 recipe (`pub(in crate::check)`
widenings + `pub(super) use self::<name>::*;` re-exports; edition 2021 permits
`poly.rs` + `poly/`). Per move: lines moved, functions moved, compile fix
rounds, cross-band coupling discovered. After all five, the member-dispatch
band (§4 probe 3's `ground.rs` shape) is re-probed in the partitioned world.
Then every resulting file is measured with the same methods as §2 (top-level
fn count, `*_error` formatters, undirected components, SCC placement,
`RefCell`/`Ordering`/`GenericTypes` sites, test attribution by call-shaped
mention), the remainder is compared against fresh house-idiom measurements
(`check.rs`/`declarations.rs`/`terms.rs`), and the verdict table asks whether
any partition variant puts every file under the bar. Everything is then
reverted; the only residue is this dossier.

Measurement tooling: the §2 method is replicated exactly and was validated
against the pristine file before any cut (178 top-level fns, 82 formatters,
8 undirected components with the same 7 strays, test boundary :12829, the
7-fn walk SCC plus the `find_bound_impl`↔`candidate_bounds_discharge` SCC —
all reproduced). Two definitional notes: formatter *body* lines here are the
sum of fn spans (signature line through closing brace), which measures
1,253 lines for the same 82 formatters the audit measured at 1,877 lines by
its span convention; and test attribution is by call-shaped mention
(`name(`), which reproduces §4's probe-1 attribution exactly (21 fns /
1,506 lines) and probe-2's (7 fns / 402 lines, counting `CrossGround` by
type-mention). Probe 3's 24 fns / 711 lines was not reproducible under any
mention rule tried (best: 17 fns / 563 lines by word-mention); its figures
below are stated under this probe's method and flagged.

### 6.1 Move 1 — `poly/unify.rs` (unification/substitution)

Moved 905 lines / 8 fns (with doc comments; §4 probe 1's same band measured
913): `compose_member_theta`, `first_unbound_sig_var`, `unify_poly_input`, its
four error formatters, `apply_subst`. poly.rs 23,540 → 22,632 (wiring block:
12 lines). Wiring: `mod unify;` + `pub(super) use self::unify::*;`.

Compile: `cargo check` needed **one fix round** — 4 widenings, not §4's 2:
`compose_member_theta`, `first_unbound_sig_var` (private → `pub(in
crate::check)`, per recipe) **plus** `unify_poly_input`, `apply_subst`
(`pub(super)` → `pub(in crate::check)`). The 11 E0425s show the §4 recipe's
re-export does **not** carry a `pub(super)` item to check-level consumers
(`combinators.rs` reaches `apply_subst` through the chain); a re-exported
`pub(super)`-of-submodule item is effectively poly-visible only. Recorded as a
correction to the §4 probe-1 wiring account. `cargo clippy -- -D warnings` ✓,
test target ✓.

Seam side: 913 lines, 1 job, 4 formatters (53 lines), 2 components (7-fn
cluster + the `poly_constructed_reference_error` orphan, called only from the
remainder — the same seam-precision miss probe 1 recorded), no SCC,
`RefCell`/`Ordering` 0 sites, `GenericTypes` 4 doc-comment mentions. Verdict
a/b/c: 0/0/0 (orphan caveat).

Remainder: 22,632 lines total, `mod tests` at :11921, 11,920 non-test lines,
170 top-level fns, 78 formatters (1,200 lines by fn-span), **16 undirected
components** (giant 153 + 15 strays, incl. a new 2-fn orphan pair
`poly_unbound_output_error`/`poly_unbound_output_ty_error` left behind —
called only by the moved `apply_subst`), **both SCCs intact** (walk cycle and
`find_bound_impl`↔`candidate_bounds_discharge`), 3+ jobs remain. Tests: 417
fns stay physically in poly.rs; 21 fns / 1,506 lines attribute to this seam by
call-shaped mention.

Cross-band coupling (this move): seam → remainder 13 edges — all leaf
formatters/renderers (`poly_type_str`, `poly_ctor_image_as_type_error`, the
`poly_unbound_output_*` pair, `explicit_*_conflict_error`,
`poly_len_conflict_error`, `poly_array_expected_error`, `poly_var_conflict_error`,
`member_unbound_variable_error`) plus `fence_quotation_literal_slot`; none
reaches either SCC. Remainder → seam: 21 caller fns, incl. the walk hub
`poly_call_term` (2 formatter calls), `resolve_user_bound` →
`compose_member_theta` (the future trait→unify edge), `poly_sig_unifies`/
`poly_sig_could_match` (the future overload→unify edge), and the remainder
impl of `CrossGround` → `apply_subst`. Cycle effect: disjoint — neither
crossed nor reduced.

### 6.2 Move 2 — `poly/instantiate.rs` (instantiation)

Moved 795 lines (347 + the 448-line `impl CrossGround`): `WordScopedRegistries`,
`ground_into_word_scoped_registries`, `arrow_stand_in`,
`discover_transitive_instantiations`, `intern_composed_bundles`,
`CrossGround` struct **and its impl block**. poly.rs 22,632 → 21,832 (wiring
block: 12 lines). Wiring: `mod instantiate;` + re-export; widenings per recipe
(`ground_into_word_scoped_registries`, `discover_transitive_instantiations`,
`WordScopedRegistries` → `pub(in crate::check)`; `arrow_stand_in` →
`pub(super)`); `CrossGround` and its impl stay private — nothing outside the
band names the type (the four CrossGround-driven strays take no `CrossGround`
in their signatures).

Compile: **one fix round, and it is a cutter defect, not a wiring one** — the
first pass moved the `CrossGround` *struct* but left its 448-line `impl` block
behind (E0412 `cannot find type CrossGround` in poly.rs, E0599 for
`impl_mono_seed`/`fixpoint` in instantiate.rs, plus a cascading E0614). The
impl was moved and all four errors cleared. `cargo check` ✓, clippy
`-D warnings` ✓, test target ✓. (Lesson for §6.7's accounting: a struct+impl
pair is one item; cutting the struct alone is a broken probe, not evidence
about the seam.)

Seam side: 802 lines, 1 job, 0 formatters, 3 undirected components —
`ground_into_word_scoped_registries` and `arrow_stand_in` mutually
disconnected islands, `discover_transitive_instantiations`+`intern_composed_bundles`
pair separate, glued only by shared types and the one instantiation story (the
§4 probe-2 shape, reproduced); no SCC; `RefCell` 6 sites, `GenericTypes` 5,
`Ordering` 0 — all confined to this job. Verdict a/b/c: 0/0/weak (3 islands).

Remainder: 21,832 lines total, `mod tests` at :11121, 11,120 non-test lines,
166 top-level fns, 78 formatters, 15 components, **both SCCs intact**, 3+
jobs remain. Tests: 7 fns / 402 lines attribute to this seam.

Cross-band coupling: seam → remainder 5 edges, **all from the `CrossGround`
impl**: `enqueue_new`, `body_calls_a_poly_word`, the cross-call error pair
(the four strays the recipe left behind), and `resolve_user_bound` (→ the
future trait band). Remainder → seam: 2 (`check_poly_combinator_standalone` →
`ground_into_word_scoped_registries`/`arrow_stand_in`);
`discover_transitive_instantiations` is still entered from check.rs:1138.
Cycle effect: disjoint; seam not a leaf.

### 6.3 Move 3 — `poly/trait.rs` (trait-obligation resolution)

Moved 1,835 lines / 31 items (29 fns + the `Position`/`PairedLeaf` enums):
`select_most_specific`, `ambiguity_error`, `bound_cycle_error`,
`candidate_bounds_discharge`, `find_bound_impl`, `resolve_user_bound`,
`check_concrete_member_site_slots`, `ctor_pin_count`,
`bare_var_impl_target_capture_error`, `ctor_image_member_site_mismatch_error`,
`unsatisfied_user_bound_error`, `unresolved_trait_obligation_error`,
`match_impl_target`, `match_impl_target_rec`, the position/pairing helper
cluster (`collect_positions` … `collect_paired_positions`), `specificity`,
`bound_set_tiebreak`, `is_strictly_more_specific`. Left behind on measured
non-membership: `fence_quotation_literal_slot` and
`member_slot_quotation_literal_error` (remainder vocabulary),
`member_unbound_variable_error` (unify's), `compose_member_theta`/
`first_unbound_sig_var` (unify's, already moved). poly.rs 21,832 → 19,969
(wiring block 14 lines + one import re-gating line).

Wiring: `mod r#trait;` + `pub(super) use self::r#trait::*;` — the file is
`trait.rs` and the keyword module name needs the raw identifier; it works
(2021 edition). 9 widenings: 6 planned (`resolve_user_bound`,
`find_bound_impl`, `specificity`, `ctor_pin_count`,
`unsatisfied_user_bound_error`, `unresolved_trait_obligation_error` →
`pub(super)`) + 3 test-driven (`Position` → `pub(super)` after 56 E0433s;
`is_strictly_more_specific` after 17; `generic_len_args_of` after 2 —
the test mod drives the specificity machinery directly). `match_impl_target`
keeps its pristine `pub(crate)` (declarations.rs:1592 reaches it through the
full path `crate::check::poly::match_impl_target`, which still resolves via
the re-export). Import drift handled: `use std::cmp::Ordering;` moved its
non-test users to trait.rs, so poly.rs's copy is re-gated `#[cfg(test)]` and
trait.rs carries its own.

Compile: 3 fix rounds (mod-file name; `Position`; the two test-called fns).
`cargo check` ✓, clippy `-D warnings` ✓, test target ✓.

Seam side: 1,867 lines, 1 job, 6 formatters (139 lines, all own vocabulary),
**1 undirected component (all 29 fns in one island)**, and the
`find_bound_impl`↔`candidate_bounds_discharge` SCC **moved whole into
trait.rs** — no SCC was split. `RefCell` 0, `Ordering` 10 sites (all the
specificity band's own), `GenericTypes` 9 sites (its own band). Verdict
a/b/c: 0/0/0 (cleanest file so far; the specificity `Ordering` sites are
one job's own vocabulary, not divergence).

Remainder: 19,969 lines total, `mod tests` at :9258, 9,257 non-test lines,
137 top-level fns, 72 formatters, 17 undirected components (giant 118 +
16 strays), walk SCC intact; the second SCC has left the remainder.
`RefCell` 4 non-test sites (21 with tests), `GenericTypes` 8, `Ordering`
1 import (now test-gated) + 1 comment mention. Tests: 38 fns / 986 lines
attribute to this band by call-shaped/type mention.

Cross-band coupling: trait.rs → unify.rs 4 (`resolve_user_bound` →
`apply_subst`/`compose_member_theta`/`unify_poly_input`;
`check_concrete_member_site_slots` → `apply_subst`); trait.rs → remainder 3
(`check_concrete_member_site_slots` → `fence_quotation_literal_slot` and
`trait_member_operand_error`; `unsatisfied_user_bound_error` →
`poly_type_str`); remainder → trait.rs 7 (`check_poly_call` →
`resolve_user_bound`; `poly_sig_could_match` → `match_impl_target`; the
member-dispatch fns `resolve_splice_member_call`/`resolve_mono_member_call` →
`find_bound_impl` + 2 formatters); instantiate.rs's `CrossGround` impl →
`resolve_user_bound` 1. Cycle effect: walk SCC untouched in the remainder;
the second SCC transferred whole — **the first cut in any probe that
reduces the remainder's cycle count** (remainder now holds 1 of the file's
2 recursion cycles).

### 6.4 Move 4 — `poly/overload.rs` (overload resolution)

Moved 338 lines / 8 items: `PolyOverloadMiss` (enum), `resolve_poly_overload`,
`no_poly_overload_matches_error`, `splice_collision_error`,
`poly_sig_unifies`, `poly_sig_could_match`, `resolve_combinator_overload`,
`no_combinator_overload_matches_error`. poly.rs 19,969 → 19,625 (wiring block
16 lines). Wiring: `mod overload;` + re-export; 3 widenings
(`poly_sig_could_match`, `resolve_combinator_overload`,
`no_combinator_overload_matches_error` → `pub(in crate::check)` — the three
terms.rs consumes at :850/:867/:888); the rest keep `pub(super)` text (poly.rs
and the test mod reach them as poly-local bindings).

Compile: 1 fix round, and it sharpened the §4 rule: a `pub(super) use
self::overload::*;` re-export fails outright (E0365, "glob import doesn't
reexport anything … no imported item is public enough") when **zero** items
qualify for the re-export's visibility; items below the bar still bind locally
in poly — they just don't travel the chain. Move 1's glob survived only
because its `pub(in crate::check)` items made the glob non-empty. `cargo
check` ✓, clippy `-D warnings` ✓ (direct run; the ambient deferred clippy
runner reported the stale pre-fix error — direct runs are authoritative per
caveat 5), test target ✓.

Seam side: 346 lines, 1 job, 3 formatters (66 lines, own vocabulary), 5
undirected components (two 2-fn pairs + 3 orphans — the band is a loose
confederation around one story, like instantiate's islands), no SCC, 0
`RefCell`/`Ordering`/`GenericTypes`. Verdict a/b/c: 0/0/weak (fragmented but
single-job).

Remainder: 19,625 lines total, `mod tests` at :8914, 8,913 non-test lines,
130 top-level fns, 69 formatters, 15 components, walk SCC intact. Tests: 0
fns / 0 lines attribute to this band by call-shaped mention (1 fn / 31 lines
by word-mention) — the band is exercised only indirectly through
`check_poly_call`.

Cross-band coupling: overload.rs → trait.rs 1 (`poly_sig_could_match` →
`match_impl_target` — the pre-existing match_impl_target caller from the
pristine caller map); overload.rs → unify.rs 2 (`poly_sig_unifies`,
`poly_sig_could_match` → `unify_poly_input`); remainder → overload.rs 4
(`check_poly_call` → `resolve_poly_overload`/`no_poly_overload_matches_error`/
`splice_collision_error`; cross-call band's `poly_cross_call` →
`no_poly_overload_matches_error`). Cycle effect: disjoint from both SCCs;
cycle count unchanged.

### 6.5 Move 5 — `poly/crosscall.rs` (cross-module call checking)

Moved 559 lines / 12 fns: `poly_cross_call`, `type_is_registered`,
`poly_cross_relate`, `poly_cross_match`, `poly_cross_output`,
`poly_cross_signature_supported`, `poly_type_mentions_caller_var`,
`poly_image_str`, `poly_growing_cross_call_error`, `poly_cross_bound_error`,
`poly_cross_var_conflict_error`, `poly_cross_call_unsupported_error`.
`poly_mentions_len_var` (inside the range but called only from the
`CrossGround` impl) stayed in the remainder, as the recipe's band-locality
rule demands. poly.rs 19,625 → 19,056 (wiring block 18 lines).

Wiring: `mod crosscall;` + **plain** `use self::crosscall::*;` — a deviation
from the §4 recipe with a reason: every item in this band was *private* in
pristine poly.rs (git-verified), nothing outside poly names any of it, so the
exact-surface transformation is `pub(super)` items + a private glob binding;
a `pub(super)` re-export cannot even be formed (zero items qualify — the
move-4 E0365 lesson). Widenings: 2 (`poly_cross_call`, `poly_cross_match` →
`pub(super)`; the first because `poly_call_term` calls it, the second for the
one test call). All other items stay private to crosscall.rs.

Compile: 1 fix round. `cargo check` ✓, clippy `-D warnings` ✓ (direct run),
test target ✓.

Seam side: 571 lines, 1 job, 4 formatters (59 lines, own vocabulary), **1
undirected component (all 12 fns)**, no SCC, 0 `RefCell`/`Ordering`,
`GenericTypes` 1 doc-comment mention. Verdict a/b/c: 0/0/0.

Remainder: 19,056 lines total, `mod tests` at :8345, 8,344 non-test lines,
118 top-level fns, 65 formatters, 15 components, walk SCC intact. Tests: 1
fn / 29 lines attribute to this band.

Cross-band coupling: crosscall.rs → overload.rs 1 (`poly_cross_call` →
`no_poly_overload_matches_error`); crosscall.rs → unify.rs 1
(`poly_cross_match` → `poly_rendered_type_mismatch_error`); crosscall.rs →
remainder 4 (`poly_copy_bound_error`, `poly_op_on_variable_error`,
`poly_type_str` ×2 — leaf formatters/renderers); remainder → crosscall.rs 1
(`poly_call_term` → `poly_cross_call` — **the walk reaches this band**, and
nothing reaches back). Cycle effect: disjoint from both SCCs; cycle count
unchanged.

Partition state after all five moves: poly.rs 19,056 lines (8,344 non-test),
holding the walk job plus member-dispatch; unify 913 / instantiate 802 /
trait 1,867 / overload 346 / crosscall 571. Five modules exist under
`src/check/poly/`, all wired with `mod` + glob and green on check, clippy
`-D warnings`, and the test target.

### 6.6 Re-probe — `poly/ground.rs` (member dispatch) in the partitioned world

§4 probe 3's band, re-cut after all five moves (its probe-3 failure was
measured against the unpartitioned remainder; `find_bound_impl`, which made
probe 3 reach a second SCC, now lives in trait.rs). Moved 1,661 lines / 17
items (the 16 fns of probe 3 + `CandidateFit`). poly.rs 19,056 → 17,381
(wiring block 20 lines). Wiring: recipe widenings exactly (2 ×
`pub(in crate::check)` for `resolve_splice_member_call`/
`resolve_mono_member_call`; 4 × `pub(super)` for `poly_trait_member_call`,
`unify_member_operand`, `render_member_decl`, `ground_member_sig_via_theta`);
everything else private.

Compile: 1 fix round — **self-inflicted, and worth recording**: cut.py's
wiring regeneration re-emitted `pub(super) use self::crosscall::*;`,
overwriting move 5's private glob and re-breaking clippy (E0365). Re-applied
the demotion; check/clippy/test all ✓ after. A real split's wiring must be
generated once, not re-derived per move.

Seam side: 1,678 lines, 1 job, 6 formatters (91 lines, own vocabulary), 2
components (11-fn splice cluster + 5-fn mono-dispatch island — probe 3's
shape), no SCC, 0 `RefCell`/`Ordering`/`GenericTypes`. Verdict a/b/c: 0/0/weak.

Remainder (with-member variant): 17,381 lines total, `mod tests` at :6670,
6,669 non-test lines, 102 top-level fns, 59 formatters, 17 undirected
components (giant 77 + 16 strays, now including the new orphan trio
`{ambiguous_trait_member_error, no_candidate_fits_operands_error,
joined_with_and}` — called only by the moved `poly_trait_member_call`), walk
SCC intact and now the remainder's only SCC. Tests: 7 fns / 270 lines
attribute to this band under this probe's method (§4 probe 3 reported 24/711
under its own, not reproducible here — see the tooling note above).

Cross-band coupling in the partitioned world: ground.rs → trait.rs 4
(`resolve_splice_member_call`/`resolve_mono_member_call` → `find_bound_impl` +
the two obligation formatters) — **the probe-3 "reaches a second SCC" finding
is now a plain cross-file call into trait.rs, and the SCC itself did not
split**; ground.rs → unify.rs 2 (`ground_member_sig_via_theta` →
`apply_subst`/`unify_poly_input`); ground.rs → remainder 14 (dominated by
`check_poly_call` ×2, `push_dispatch_outputs` ×2, leaf formatters, and the
orphan trio it orphaned); remainder → ground.rs 1 (`poly_call_term` →
`poly_trait_member_call`). The bidirectional remainder↔ground coupling that
disqualified probe 3 persists in direction (remainder still calls the seam's
entry points from the walk, seam still calls `check_poly_call`), but at file
granularity it is now one edge-pair between siblings rather than coupling
through the walk's interior.

### 6.7 Final partition measurement (with-member variant, all six moves in)

Same methods as §2, run over every resulting file. Dep sites are counted
`code/comment/test` (comment = line's stripped form starts with `//`; test =
inside the `mod tests` region, which stays physically in poly.rs). Test
attribution is by call-shaped fn mention + word type mention, computed once on
the final tree; the seven attributions sum to 418 because one test fn mentions
two bands (counted in both, as in §4). The remaining 347 test fns / 7,698
lines attribute to the remainder.

| file | non-test | test attrib. | fns | `*_error` (lines) | comps (sizes) | walk SCC | 2nd SCC | RefCell c/m/t | Ord c/m/t | GT c/m/t | verdict a/b/c |
|---|---|---|---|---|---|---|---|---|---|---|---|
| poly.rs remainder | 6,669 | 347 / 7,698 | 102 | 59 (845), :3721–:6566 | 17 (77 giant + 16 strays) | **whole, here** | gone (moved) | 3/1/17 | 1*/1/20 | 4/3/22 | **fires / fires / fires** |
| poly/unify.rs | 913 | 21 / 1,506 | 8 | 4 (53) | 2 (7, 1 orphan) | – | – | 0/0/0 | 0/0/0 | 0/4/0 | 0 / 0 / 0 |
| poly/instantiate.rs | 802 | 4 / 292 (word: 7 / 402) | 4 | 0 | 3 (2, 1, 1) | – | – | 6/0/0 | 0/0/0 | 5/0/0 | 0 / 0 / weak |
| poly/trait.rs | 1,867 | 38 / 986 | 29 | 6 (139) | **1 (29)** | – | **whole, here** | 0/0/0 | 10/0/0 | 8/1/0 | 0 / 0 / 0 |
| poly/overload.rs | 346 | 0 / 0 | 7 | 3 (66) | 5 (2, 2, 1, 1, 1) | – | – | 0/0/0 | 0/0/0 | 0/0/0 | 0 / 0 / weak |
| poly/crosscall.rs | 571 | 1 / 29 | 12 | 4 (59) | **1 (12)** | – | – | 0/0/0 | 0/0/0 | 0/1/0 | 0 / 0 / 0 |
| poly/ground.rs | 1,678 | 7 / 270 | 16 | 6 (91) | 2 (11, 5) | – | – | 0/0/0 | 0/0/0 | 0/0/0 | 0 / 0 / weak |

\* the remainder's one non-test `Ordering` code site is the `#[cfg(test)]`
import line itself (its only non-test *use* moved to trait.rs); excluding the
import the remainder has 0 code sites + 1 comment mention.

Named jobs per file: unify = unification/substitution (1). instantiate =
instantiation (1). trait = trait-obligation resolution incl. specificity
ranking (1). overload = overload resolution (1). crosscall = cross-module call
checking (1). ground = member-call signature grounding/dispatch (1).
Remainder = **at least 5 still distinguishable jobs**: the abstract walk
(`check_poly_body` :820 → `poly_walk` :936 → `poly_term` :1006 →
`poly_call_term` :1146 → `poly_eliminator_call` :2429 → `poly_walk_arms`
:2952 → `poly_combinator_call` :3357, with the quotation-literal and
combinator-entry fns); generic construction/elimination
(`poly_construction_header` :3656, `poly_bind_construction_arg` :3736,
`poly_destructure_generic` :4011, `poly_construct_generic` :4088,
`poly_reference_word` :4320, the arm/row machinery :2878–:3336); copy/borrow/
slice gating (`poly_is_copy` :264, the `PolyScope`/`PolyBorrow`/`PolySlot`
plumbing, `check_poly_array_index` :4546, `check_poly_slice_offset` :4583,
`poly_copy_gate` :4605, `check_poly_reference_across_back_edge` :2126); the
dispatch hub and its glue (`check_poly_call` :4762, `push_dispatch_outputs`
:5283, `strip_stand_in_tag` :5482, `fence_quotation_literal_slot` :5379 and
the orphaned formatter trio :5640–:5716); and the instantiation strays the
recipe left behind (`enqueue_new` :5310, `body_calls_a_poly_word` :5329, the
cross-call error pair :5341/:5361, `poly_mentions_len_var` :2069) — plus the
shared type plumbing (`TraitCtx`/`CrossCtx`/`TraitResolveCtx`,
`poly_type_str` :6592).

Formatter interleaving in the remainder is still structural: 59 formatters
span :3721–:6566, sitting between and inside the walk/construction/gating fns
(e.g. `poly_generic_field_len_unbound_error` :3721 between
`poly_construction_fallback` :3688 and `poly_bind_construction_arg` :3736;
the borrow-error band :5812–:5981 inside the gating vocabulary; the late band
:6376–:6566 trailing `poly_type_str`'s callers).

SCC placement: the 7-fn walk SCC is whole and solely in poly.rs; the
`find_bound_impl`↔`candidate_bounds_discharge` SCC is whole and solely in
trait.rs. No recursion cycle spans a file boundary. File-level module cycles
do exist after the partition (remainder ↔ trait.rs, remainder ↔ ground.rs,
and the remainder → crosscall → overload → trait → unify → remainder chain)
— ordinary Rust module cycles, not recursion cycles, but they are coupling
the partition created or preserved.

### 6.8 House-idiom comparison (fresh measurements)

| file | total | non-test | test region | fns | `*_error` (lines) | comps (largest) |
|---|---|---|---|---|---|---|
| check.rs | 5,301 | 3,462 | 1,839 | 84 | 42 (473) | 18 (61) |
| declarations.rs | 4,571 | 2,163 | 2,408 | 73 | 28 (279) | 14 (14) |
| terms.rs | 5,809 | 3,899 | 1,910 | 51 | 18 (232) | 1 (51) |

The house idiom itself is not component-clean (check.rs: 18 undirected
components, 42 formatters) — but every house file is ≤ 3.9k non-test lines
with ≤ 84 top-level fns. The partitioned remainder (6,669 non-test, 102 fns,
59 formatters) is still the largest stage file in the checker on every axis —
roughly 1.7–1.9× the biggest house file in non-test lines and fn count, and
1.4× check.rs's formatter count — while each moved band now sits at or below
house scale (largest: trait.rs 1,867 lines / 29 fns / 6 formatters).

### 6.9 Partition verdict

Rule under test (spec, D1/D2): a cut qualifies only if **both sides < 2 firing
signals** and the cut **reduces rather than crosses** the recursion cycle.

| variant | moved files (a/b/c) | remainder (a/b/c) | cycle | every file < 2? |
|---|---|---|---|---|
| 5 moves (unify, instantiate, trait, overload, crosscall) | all ≤ weak (0/0/0 or 0/0/weak) | remainder 8,344 non-test, 130 fns, 65 formatters, 15 comps → **3 firing** | both SCCs whole, uncrossed | **No** — remainder 3 |
| 6 moves (+ ground member dispatch) | all ≤ weak | remainder 6,669 non-test, 102 fns, 59 formatters, 17 comps → **3 firing** | walk SCC whole in remainder; 2nd SCC moved whole to trait.rs | **No** — remainder 3 |

**The hypothesis is refuted on the measured evidence.** The job partition
moved 6,093 lines of non-test code (48% of poly.rs's non-test mass) out of
poly.rs, produced five clean single-job modules, moved the second recursion
SCC whole, and split no cycle — and the remainder still fires all three
signals, because the walk cluster, its construction/elimination machinery,
the copy/borrow gating, the dispatch hub, the shared type plumbing, and the
formatter vocabulary that serves them are one interlocking mass that no job
boundary separates. Each individual move satisfies the D1 test against the
*other* files; the conjunctive test fails on poly.rs itself in every variant
measured. Under the pre-committed rule, no partition variant qualifies for
D2; this dossier records the measurement and makes no ruling.

### 6.10 Revert verification

After §6.9 was recorded, the entire partition was reverted:
`git checkout -- src/check/poly.rs && rm -rf src/check/poly`. Verified:
`git status --porcelain` shows only the untracked dossier
(`?? docs/roadmap/poly-split-ruling-audit.md`); HEAD is `575a5a1f9a23bc76b086cf4267b923edcde0cac1`;
`src/check/poly.rs` is 23,540 lines (pristine); no `src/check/poly/` directory
exists; `cargo check` is green on the restored tree. No commit was made; no
test, golden, or diagnostic text changed at any point in this probe round.

### 6.11 Summary of corrections this probe round contributes

1. **§4 probe-1 wiring account corrected**: the unify move needed 4
   visibility widenings, not 2 — a `pub(super) use self::X::*;` re-export
   does not carry a `pub(super)`-of-X item to check-level consumers (11
   E0425s from `apply_subst`/`unify_poly_input` callers in combinators.rs).
2. **Re-export mechanics pinned down** (moves 4/5): a `pub(super)` glob
   re-export fails outright (E0365) when zero items are visible enough;
   items below the bar still bind locally in poly. A band nothing outside
   poly consumes is exactly wired with a private `use self::X::*;`.
3. **A struct+impl pair is one cut unit** (move 2's fix round): cutting the
   `CrossGround` struct without its 448-line impl is a broken probe, not
   evidence about the seam.
4. **`mod r#trait;` works** for a `trait.rs` file under the 2021 edition
   (raw-identifier module name resolves to `trait.rs`).
5. **§4 probe-2's attribution number (7 fns / 402 lines) is word-mention**;
   under the call-shaped convention calibrated on probe 1 it is 4 / 292.
   Probe 3's 24/711 could not be reproduced under any mention rule (best
   17/563 word-mention; this probe's method gives 7/270).
6. **Cross-band edge directions recorded** (§6.1–§6.6): walk → crosscall
   (one-way), crosscall → overload, overload → trait, trait → unify,
   ground → trait/unify/remainder, instantiate's impl → trait/unify/strays —
   a layered call DAG into the walk side, with remainder↔ground and
   remainder↔trait the only mutual pairs.
