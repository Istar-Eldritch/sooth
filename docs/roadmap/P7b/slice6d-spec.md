# P7b.S6d — Slices as Iterator targets (the built-in-view lift)

Specification. Base: clean tree at HEAD `390b1b2` (worktree `soo-38`, branch
`soo-38`; full suite re-verified 3480/0 in the paper-test round). Tracking
issue: **SOO-38** (SOO-1 is the older duplicate, close it when this lands).

Frozen companion docs (read them; this spec cites them by name and does not
re-derive their evidence):

- [slice6d-brief](./slice6d-brief.md) — problem, DQ1–DQ5, the four walls,
  scope/out-of-scope. Pre-PREREQ; its wall ledger is superseded by the probes
  where they disagree.
- [slice6d-probes](./slice6d-probes.md) — rounds S6d-1..S6d-10. **S6d-7..S6d-10
  (run 260911, committed `40aacce`) are the current-tree evidence base.**
- [slice6d-paper-tests](./slice6d-paper-tests.md) — the validated golden set
  **G1–G22**, the join/back-edge desk-checks (verdicts A–E), the units list,
  per-item risks/fallbacks. Committed `e1fa5c8`. This spec's success and phase
  exit criteria are expressed in these goldens.
- [slice6d-prereq](./slice6d-prereq.md) — the shipped PREREQ capability whose
  NFRs and Rulings A–F bind this slice.
- [probes/s6d_baseline.md](../../../probes/s6d_baseline.md) plus the committed
  `probes/s6d_*.sth` fixtures — byte-exact clean-tree behaviour. **Already
  committed (`40aacce`, `e1fa5c8`); implementation does not re-derive them.**

## Problem

`Slice['T]`/`!Slice['T]` have existed since P7.S3c as runtime-length views:
`Type::Slice`, interned per `(element, mutable)` with no count, with `slice`/
`subslice`/`len`/`&>`/`&!>` words and one instantiation over all lengths.
`impl: Iterator for Slice[i64]` parses as an impl target, but `next`'s
App-headed member row (`'It['T] -- Step['T 'It['T]]`) hits the S2-6
concrete-target fence: `member_app_concrete_target_error`
(`src/ast.rs:2135`, raised via `fence_member_app_against_concrete_target`,
`src/ast.rs:2208`). A built-in slice has no ctor header for
`ImplTarget::is_mono_ctor_app` (`src/ast.rs:2569`) to recognize, so the P7b.S8
Range lift (`impl_target_pattern_poly_type`, `src/parser.rs:4242`, keyed on a
`GenericId` into the struct/enum registries) does not transfer: `Type::Slice`
comes from `intern_slice_type`, keyed structurally on `(element, mutable)` with
no header. The generic spelling `Slice['T]` fails even earlier
(`unknown type 'T`, S6d-3) and is not a workaround.

The PREREQ slice (`aa590d2`/`613c301`/`f27c34d`) closed the deeper wall: a
`Step[i64 Slice[i64]]` monomorph now lays out (two-word slice slot), the
storage/return bans are located, and the seven in-frame deriv/alias propagation
sites are live. What remains before slices dispatch through `Iterator` is this
slice's residual work: the DQ4 fence-lift grounding, a per-impl `inline`
spelling, a pre-existing generated-enum clobber fix, a borrow-join extension,
and the library impl. All of it is checker / target-grammar; **no `src/ir/`
change** (the roadmap's own S6d line said `S-M`, checker/target-grammar only;
the delivered size is **M** — see the roadmap correction below).

## Design frame (decided — each requirement cites its evidence round)

Settled by the probe evidence and the paper-test desk-checks. These are
recorded here as decided requirements, not open questions.

### REQ-1 — Shared `Slice[i64]` target only

The impl lands for shared `Slice[i64]`. The mutable `!Slice[i64]` target is
**out of scope**: Ruling A's enum-payload sweep fires first at the `Step`
declaration span (S6d-9, pinned by **G9** and its always-true twin
`s6d_b3_mut_payload.sth`). Deferral pointer: **SOO-45**. No mixed-mutability
`next` shape exists to design for (DQ2, S6d-5).

### REQ-2 — Fence lift = the two-half sentinel patch

Exactly as evidenced in S6d-8.1. Two halves:

- **Parser half.** `SLICE_SENTINEL_IDX` + `slice_sentinel` +
  `rewrite_slice_sentinel` at `src/parser.rs:855-964`; the slice branch inside
  `parse_impl_member_body`, inserted **immediately before the
  `is_mono_ctor_app` branch** (`src/parser.rs:4519` on this tree, right after
  the `dg` construction at `:4485-4497`), so it preempts **both** the
  `is_mono_ctor_app` arm (`:4519`) and
  the `is_concrete` arm (`:4558`). (The paper tests' probe coordinates,
  `:4629-4691`, are post-insertion/spike-tree line numbers from S6d-8.1 item
  3, which is explicit that the branch lands "BEFORE both" arms — that intent
  is binding, not the literal numbers.) A fake `PolyType::Generic` carries the
  slice's element and is rewritten back to `Concrete(Type::Slice(..))` before
  any code reads its bogus header. This is a momentary, documented violation
  of `PolyType::Generic`'s own invariant: document the exception with
  **mutual pointers** at the construction site and at the invariant doc
  (`src/ast.rs:2711`).
- **Dispatch half.** The slice guard arm in `resolve_mono_member_call`,
  `src/check/poly/ground.rs:1319-1333`, reading the already-grounded member
  word.

`PolyType::SliceApp` (a dedicated variant touching every exhaustive `PolyType`
match) is the **recorded fallback if the invariant exception is ruled
unacceptable** — bigger, unverified; not the plan (S6d-2 ledger).

### REQ-3 — Per-impl `inline` spelling

Impl members gain an optional `inline` keyword. `declares_inline` = the impl's
spelling when present, else the trait member's (currently inherited
unconditionally at `src/parser.rs:4449-4453`). **Hard requirement, no spelling
fallback:** the member output ban (**G10**) forces the member to be inline, and
trait-level inline is **NOT** acceptable — it breaks 6 List/Range goldens via
the inline-poly-member nullary-ctor mis-resolution (S6d-8.5). Those 6 goldens
(the **G22** canaries) are the pin that trait-level behaviour stays unchanged.
If the desugar stalls, the slice stalls: escalate (F3).

### REQ-4 — The Step-monomorph clobber fix

A **pre-existing S8b-class bug**, clean-tree reproducible (`s6d_m`): a bare
`Step[i64 Slice[i64]]` signature mention re-types List consumers' `More>`
resolution. **Required** because the placement gate (**G19**) forces the slice
impl into `lib/core/iterator.sth` (`Slice` declares no module, so the
co-declaration arm cannot apply and the range.sth beside-the-protocol
convention is structurally unavailable for built-in-view targets), and
co-location mints the `Step[i64 Slice[i64]]` monomorph for every importer.

Fix surface (re-diagnosed against measurement, not the paper tests' original
guess): candidate **visibility**, not span-keying. The bare-name candidate
lookup's env-hit arm (`src/check/terms.rs:966-968`, the `Some(v) =>
v.as_slice()` arm of the `env.get(name)` match, reached when `scoped_ops` is
`None`) returns only the single parse-time-minted monomorph and never
consults `mint_fallback_candidates` (`terms.rs:2249-2289`, which only runs on
the sibling `None =>` / env-miss arm) — so a second, check-time-minted
monomorph of the same generated enum is invisible to it. Repro: `s6d_m`
fails with the `More>` mismatch; adding one parse-time mention of
`Step[i64 List[i64]]` to the same program (making both monomorphs
env-visible) builds clean. **The fix is to union the env-hit candidates with
the live check-time mints at that lookup**, keyed per-monomorph so each
bare-name site still resolves to its own. `select_overload_fallback_sourced`
(`src/check/builtins.rs:180-199`) is **exonerated**: it operand-filters
first (`:185-190`), so a wrong-shaped monomorph cannot survive its match —
do not touch it. The existing span-keyed bookkeeping arms
(`src/check/terms.rs:1124-1157`, `:1210`) are candidate-recording precedent,
not the clobber site. The spec pins the invariant plus **G7/G8**.

### REQ-5 — Borrow-join union + `back_edge_outs` deriv forwarding, TOGETHER

- **Union at BOTH join sites** (A-amend 1): `check_branch_join`
  (`src/check/terms.rs:3607`, error at `:3613`) **and** `merge_arm_output_slot`
  (`src/check.rs:2718`, deriv match at `:2725-2736`). Both carry the same
  refusal rule; patching only one leaves a `Step?`-dispatch-shaped state merge
  (the while drain's inner shape) refusing.
- **Condition: `owned_root.is_none()`** on the one-sided deriv (A-amend 2) — the
  same predicate the back-edge guard uses for its accept-case
  (`src/check.rs:1677-1692`) and `carried_borrow` for its no-contention case
  (`src/check.rs:3164`). Rooted one-sided asymmetries and two-root disagreements
  stay refused **byte-identically** (**G4/G5**).
- **`back_edge_outs` forwards `deriv` alongside `surviving`**
  (`src/check/terms.rs:3784`, inside `back_edge_outs`, `:3773-3790`). `None→None`
  for every existing golden; the PREREQ guard test
  (`back_edge_outs_forwards_surviving_set_along_index_map`) is untouched, and its
  twin `back_edge_rejects_a_deriv_carrying_aggregate_argument` still rejects at
  the call site (the guard scans arguments before `back_edge_outs` runs).

**Justification (why together):** the no-silent-mask soundness argument. The
join-union alone would turn `while`'s loud back-edge error into a silent
laundering channel for any shape that cannot recover the deriv from a sibling
arm (verdict C). Landing both closes that hole. **Mechanism fact (corrected):**
naming does **not** always mint a rootless reborrow — `Provenance::reborrow`
inherits the root when one exists (`src/check/engine.rs:418`, `let owned_root
= held.and_then(|id| self.deriv(id).owned_root.clone())`). Naming is rootless
only when the binding carries no held deriv in the first place — a
parameter-seeded slice slot is `Slot::computed` (`src/check/word_entry.rs:
219-221`, deriv-free), so *that* mint is rootless. A local rooted in a frame
place preserves the root through a reborrow, exactly as the spec's own **G4**
fixture (`probes/s6d_j`, naming `va`) demonstrates: the refusal reads "a
borrow of `a`" — a rooted result. Rootless derivs are common in slice
consumer code (parameter-rooted remainders are the typical shape) but not
universal, and **no laundering spelling is known**: a rooted deriv survives a
reborrow, so a rooted one-sided asymmetry (G4) cannot be re-spelled into the
union's rootless case by naming a local. The residual case — a rooted view
passed into a helper, which re-mints rootless in the callee frame
(`Slot::computed`) — is the already-accepted back-edge shape (G16), not a
join-site channel, so it is not a laundering route either. G4/G5 stay
reachable guardrails, not vestigial ones.

**Fallback (F5), if either half stalls:** the as-done poly-helper spelling
(`probes/s6d_a_fence_baseline.sth`'s impl body) with join/back_edge untouched —
probe-proven end-to-end on the sentinel-patched tree (S6d-8.2/8.3). If used:
G2/G3's post-fix expectations change to that body, and **G6 is withdrawn** (the
while drain stays closed; it is not the exit criterion, G8 is). Trait-level
inline is not a fallback for any of this.

### REQ-6 — The library impl in `lib/core/iterator.sth`

`impl: Iterator for Slice[i64]` with `: next inline ... ;` and the **natural**
body over `len`/`&>`/`subslice`/`More`/`Done` (no as-done once REQ-5 lands; the
exact spelling is in the paper tests G1/G2, with `s6d_a`'s body as the
fallback). `for_each`/`fold` are **NOT** touched: bound-generic consumers stay
closed at the `'It['T]` slot unification (no ctor head; **G20**; SOO-60
territory, unreached this slice; see also SOO-40's interface note below).
Mono consumers only: self-tail recursive drain
(**G16**, passes — parameter-rooted remainders have no `owned_root`), non-tail
recursive drain (**G18**), unrolled bounded stepping; the while-threaded drain
becomes expressible once REQ-5 lands (**G6**, predicted stdout `6\n6\n6\n` —
desk-check C).

## Non-functional requirements (PREREQ NFRs restated as binding)

- **NFR-1 (no carve-outs, no protocol fork).** No `Type::Slice` carve-out in
  `contains_reference`; no second `Iterator` protocol. The sentinel is a
  grounding-path device, not a predicate relaxation. One protocol, S8's R1 not
  re-litigated.
- **NFR-2 (backend-neutral IR).** No `src/ir/` change in this slice at all.
  `Ptr[T]` stays opaque; the two-word slice slot already lands (PREREQ REQ-1).
- **NFR-3 (safety monotonicity).** Every intermediate commit is at least as safe
  as the base. The bans and refusals that exist today stay **byte-identical** in
  every intermediate commit: **G4/G5/G9/G10/G11/G17/G19/G20/G21** are the
  regression pins, plus the 6 List/Range canaries and the PREREQ guard test.
  Nothing admits a slice-bearing value into an escaping position at any step.
- **NFR-4 (no regression, PREREQ-restated).** Every PREREQ-shipped admission
  (the `41\n5\n` **G14** capture, the shared two-word slice layout, the seven
  propagation sites) stays exactly as admitted; this slice only widens
  dispatch, never re-derives PREREQ's own rulings.

## Success criteria (observable, named goldens)

Exit criterion: **G8** (`list_and_slice_impls_drain_through_one_imported_protocol`)
— one program, both lib impls, bare `next` dispatching to each; stdout
`1\n2\n3\n3\n3\n3\n`, exit 0. It exercises items 1+2+3+4+6 together through the
exported surface.

Full pin list (paper tests §"The golden set"; "today" = clean tree at HEAD
`390b1b2`, "post-fix" = the expected golden):

| G | name | today | post-fix |
| --- | --- | --- | --- |
| G1 | `slice_impl_member_per_impl_inline_builds_and_runs` | fence | `3\n4\n` |
| G2 | `natural_next_body_checks_with_the_rootless_join_union` | fence | `3\n4\n` |
| G3 | `rootless_join_asymmetry_refused_today_carried_by_the_union` | join refusal | `build_ok` |
| G4 | `rooted_join_asymmetry_stays_refused_byte_identically` | refuse (names `a`) | **byte-identical** |
| G5 | `two_root_join_asymmetry_stays_refused_byte_identically` | refuse (b/a) | **byte-identical** |
| G6 | `while_threaded_slice_drain_runs_once_back_edge_outs_forward_deriv` | fence | `6\n6\n6\n` |
| G7 | `clobber_touch_mint_leaves_list_drain_resolutions_untouched` | type mismatch | `1\n2\n3\n` |
| G8 | `list_and_slice_impls_drain_through_one_imported_protocol` | no-dispatch | `1\n2\n3\n3\n3\n3\n` (**exit criterion**) |
| G9 | `mutable_slice_impl_first_firing_is_the_enum_payload_sweep` | fence | Ruling A sweep error |
| G10 | `noninline_slice_member_output_ban_holds_post_fix` | fence | member output ban |
| G11 | `fence_byte_stability_for_a_non_slice_concrete_target` | fence (`Unit`) | **byte-identical** |
| G12 | negative control (ii): dispatch half | — | **manual spike** (not a suite golden) |
| G13 | `trait_level_inline_spelling_still_builds_for_a_probe_local_slice_impl` | fence | `3\n4\n` |
| G14 | `prereq_admissions_stay_byte_green` | `41\n5\n` | `41\n5\n` |
| G15 | `monomorphic_consumer_drains_two_views_end_to_end` | fence | `3\n3\n3\n3\n3\n9\n` |
| G16 | `selftail_drain_passes_the_back_edge_guard_with_a_parameter_rooted_remainder` | fence | `3\n3\n3\n3\n3\n` |
| G17 | `selftail_inline_drain_still_rejected_at_the_back_edge_guard` | fence | back-edge guard error |
| G18 | `nontail_drain_runs_with_one_frame_per_element` | fence | `4\n4\n4\n4\n` |
| G19 | `slice_impl_over_an_imported_trait_still_must_live_in_the_declaring_module` | fence | placement gate error |
| G20 | `bound_generic_consumers_stay_closed_at_the_slot_unification` | fence | slot-unification rejections |
| G21 | `times_hosted_slice_drain_stays_closed_at_the_abstract_row` | fence | abstract-row no-dispatch |
| G22 | `existing_suite_green` + the 6 canaries | 3480/0 | 3480 + new / 0 |

Error goldens are byte-exact (measure-then-pin from the live patched binary per
house convention; the message content is captured verbatim in the probes doc at
S6d-8.4(iii), S6d-9, S6d-8.6).

## Scope and boundaries

**In scope:** REQ-1..REQ-6 above and their goldens/units. **REQ-1 is
scope-only** (a target-shape decision, not a code change): it has no phase of
its own, because "shared only" falls out of the existing Ruling A sweep
firing first (G9) — the phase plan below implements REQ-2..REQ-6.

**Out of scope / non-goals (each with its note):**

- **No `src/ir/` change.** The roadmap's own S6d line said checker/target-grammar
  only; the delivered size is **M** — the roadmap entry needs a correction at
  implementation time (house convention: a docs commit lands with the phase
  work; see below).
- **`!Slice[i64]` mutable target** — Ruling A's sweep fires first (G9). SOO-45.
- **`Slice['T]` generic-element targets** — unreachable spelling (S6d-3), not a
  deferral. Ticketed anyway: **SOO-48**.
- **`for_each`/`fold` and any bound-generic consumer** — closed at the `'It['T]`
  slot unification (G20). SOO-60 (the probes narrowed its scope to the poly-body
  App-dispatch output loss; worth noting on the ticket at implementation time).
- **The D-caveat:** a member call *inside a poly combinator splice* re-walks onto
  the S11 strict-grounding wall (`inline_combinator`,
  `src/check/combinators.rs:313`; `consumer_expected_type`'s spliced-body
  half declines for a mono member, `src/check/terms.rs:2649`). No golden
  uses that shape; `next`-inside-a-quotation is separately closed for slices
  (S6d-10.4 / G21). Non-goal with a note.
- **The diagnostic cosmetic:** `mono_member_no_dispatch_error` renders an
  abstract operand row as EMPTY (`the operand types here are `` ` ``). Noted for
  whoever touches it, not this slice.
- **The instantiation-audit interface note:** when SOO-40 lands, its blanket
  instantiation-time poly-signature audit must keep **exempting poly ctor-shaped
  (synthesized multi-output bundle) outputs** — the as-done lesson in the probe's
  Notes; a blanket audit otherwise strands the PREREQ's return-bundle ABI.

## Advisory solution approach

The five code changes are code-independent except the lib impl, which depends
on all four; their **automated goldens interlock pairwise** (see P1/P2 below)
even though the code does not. Recommended order mirrors the phase plan: land
the parser desugar (REQ-3) and the two-half sentinel (REQ-2) first (they
unblock any live exercise), the clobber fix (REQ-4) and the join/back-edge
pair (REQ-5) in parallel, then the lib impl + consumer goldens (REQ-6) last.
Every changed site gets the paper tests' named units (§Units; 15 named, see
the unit count note under Housekeeping). Re-run the growth signals (CLAUDE.md)
at phase exit against every file this slice grew — especially
`src/check/terms.rs` (join sites + back_edge + clobber channel, 5809 lines,
taking P3+P4) and `src/parser.rs` (sentinel helpers + desugar, 15944 lines,
taking P1+P2); split only if 2+ signals fire together. P2's and P4's phase
Exits repeat this check explicitly.

## Codebase map (path:line anchors verified against this tree)

Parser (REQ-2, REQ-3):

- `src/parser.rs:855-964` — `SLICE_SENTINEL_IDX`, `slice_sentinel`,
  `rewrite_slice_sentinel` (new helpers).
- `src/parser.rs:4519` — the slice branch's insertion point in
  `parse_impl_member_body`, immediately before the `is_mono_ctor_app` branch
  and just after the `dg` construction at `:4485-4497`; it preempts **both**
  `is_mono_ctor_app` (`:4519`) and `is_concrete` (`:4558`). (The paper tests'
  `:4629-4691` are spike-tree coordinates; see REQ-2.)
- `src/parser.rs:4449-4453` — the current unconditional `declares_inline`
  inheritance (REQ-3 makes it impl-spelling-first).
- `src/parser.rs:4242` — `impl_target_pattern_poly_type` (S8's Range route,
  reference only; not the slice path).

AST (REQ-2 invariant exception):

- `src/ast.rs:2711` — `PolyType::Generic` invariant doc (add the mutual
  pointer).
- `src/ast.rs:2135` — `member_app_concrete_target_error` (the fence text;
  byte-stable for non-slice targets, G11).
- `src/ast.rs:2208` — `fence_member_app_against_concrete_target`.
- `src/ast.rs:2569` — `ImplTarget::is_mono_ctor_app` (unchanged; slices never
  satisfy it — that is why the sentinel exists).
- `src/ast.rs:2271` — the panic G12's manual revert spike (negative control
  (ii)) reaches if the dispatch arm is reverted.

Dispatch grounding (REQ-2 dispatch half, REQ-6 call sites):

- `src/check/poly/ground.rs:1319-1333` — the slice guard arm in
  `resolve_mono_member_call` (reads the already-grounded member word).
- `src/check/poly/ground.rs:1273-1442` — the mono member-call branch
  (sig-check, span-keyed record at `:1423`, `push_dispatch_outputs` at `:1424`);
  a mono call is a sig-check, not a body re-walk (verdict D).
- `src/check/combinators.rs:313` — `inline_combinator` (the D-caveat splice
  path; non-goal).

Join / back-edge (REQ-5):

- `src/check/terms.rs:3607-3620` — `check_branch_join`, error at `:3613`,
  `borrow_join_disagreement_error` at `:3797`. Union site 1.
- `src/check.rs:2718-2754` — `merge_arm_output_slot`, deriv match at
  `:2725-2736`. Union site 2 (A-amend 1).
- `src/check.rs:1677-1692` — the back-edge guard body, whose `owned_root: None`
  arm is the union's condition predicate (`check_reference_across_back_edge`).
- `src/check.rs:3164` — `carried_borrow`'s no-contention case (same predicate).
- `src/check/terms.rs:3773-3790` — `back_edge_outs`; copy `deriv` alongside
  `surviving` at `:3784`.
- `src/check/terms.rs:820-827` — the self-tail back-edge caller arm.
- `src/check.rs:1670` — `check_reference_across_back_edge` (the argument-scan
  guard; untouched, G16/G17 pin it).
- `src/check/terms.rs:248` — the name-read push reborrow arm; rootless only
  when the naming's `held` deriv is itself absent (see the REQ-5 mechanism
  correction).
- `src/check/word_entry.rs:219-221` — `Slot::computed` seeds a deriv-free
  parameter slot (the source of rootlessness for parameter-rooted remainders).
- `src/check/engine.rs:418` — `Provenance::reborrow`'s `owned_root` line: a
  held deriv's root is inherited, not erased.
- `src/check/word_families.rs:252-256` — the exclusivity-scan predicate a kept
  deriv feeds.

Clobber fix (REQ-4):

- `src/check/terms.rs:966-968` — the env-hit candidate arm (the actual fix
  site: union with live check-time mints here, per-monomorph).
- `src/check/terms.rs:2249-2289` — `mint_fallback_candidates` (the env-miss
  arm's existing supplier; the fix brings its candidates into the env-hit arm
  too).
- `src/check/builtins.rs:180-199` — `select_overload_fallback_sourced`
  (exonerated: operand-filters first at `:185-190`; do not touch).
- `src/check/terms.rs:1124-1157`, `:1210` — the existing span-keyed
  bookkeeping arms (recording precedent, not the clobber site).

Builtins / word families (reference — already work, PREREQ-shipped):

- `src/check/builtins.rs:536` — `Type::Slice(_, mutable, _) => !mutable` (Copy).
- `src/check/word_families.rs:32-63` — `&>`/`&!>`; `:926` (`subslice`), `:1104`
  (`len`, the `Type::Slice` arm) — mutability-preserving.

Library (REQ-6):

- `lib/core/iterator.sth` — add `impl: Iterator for Slice[i64]` with
  `: next inline ... ;` (natural body). Current imports are only
  `import: intrinsics | drop | ;` and `import: self::list | List Nil Cons | ;`
  (lines 13-14); the natural body needs widening — `len sub >usize
  sub dup swap` from `intrinsics`, `if`/`eq` from `self::prelude` (cf.
  `lib/core/range.sth:6-8`'s import pattern; `subslice`/`&>`/`@` are not
  import-gated — they are always-dispatched builtins, `parser.rs:4065-4070`).
  `core::iterator` deliberately
  does not export bare `next`.

## Open questions and risks (adapted from paper tests §Risks and fallbacks)

- **F1 (shared only).** No risk: the mutable rejection is Ruling A's sweep,
  first-firing (G9); the `s6d_b3` twin catches a span/wording shift. SOO-45.
- **F2 (sentinel patch).** Negative controls cover it: (i) G11 byte-stability for
  non-slice targets, (ii) G12, a **manual implementation-time spike** (not a
  suite golden — captures the `ast.rs:2271` panic under a reverted dispatch
  arm), (iii) G10 the output ban. If the invariant exception is ruled
  unacceptable, the fallback is `PolyType::SliceApp` — a bigger, unverified
  diff (S6d-2), not a spelling fallback.
- **F3 (per-impl inline).** Hard requirement, no spelling fallback. Trait-level
  inline is NOT acceptable (breaks the 6 canaries, G22). If the desugar stalls,
  the slice stalls: escalate.
- **F4 (clobber fix).** The gate leaves no other home for the impl (G19), so a
  stall ships nothing user-visible. Fix surface is the env-hit candidate arm's
  visibility (`terms.rs:966-968`), not span-keying (see REQ-4's correction);
  union, don't widen the fallback picker. Pins: G7, G8, plus the two existing
  suite canaries at `tests/phase6_slice3b.rs:208` and
  `tests/phase7_slice12.rs:681`.
- **F5 (join + back_edge, together).** Probe-proven fallback: the `s6d_a` as-done
  body with join/back_edge untouched (S6d-8.2/8.3). If 5(a) stalls: G2/G3 change
  to the as-done body, G4/G5 stay frozen. If 5(b) stalls: G6 is withdrawn (not
  the exit criterion). Trait-level inline is not a fallback here (F3).
- **F6 (lib impl).** for_each/fold stay out (G20). The natural body (G2) depends
  on 5(a) and verdict D's mono-caller finding; the as-done body is the spelling
  fallback if the S11-splice caveat ever matters.

## Housekeeping

- Probe fixtures are already committed (`40aacce`, `e1fa5c8`) — implementation
  reuses them as-is, does not re-derive. Note: `probes/s6d_baseline.md` itself
  covers only the 13 round-1 fixtures (landed in `40aacce`, not amended by
  `e1fa5c8`); the 8 round-2 fixtures' (`s6d_j`..`s6d_q`) byte-exact captures
  live only in `slice6d-paper-tests.md`.
- Goldens land in the `tests/phase7b_slice8.rs` successor convention (look at how
  slice goldens are organized there; keep `single_file_hosted` /
  `build_ok` / `build_run_keep` / `build_error_located`). Harness note: the
  harness prepends `import: intrinsics * ;` + `import: hosted::show | . | ;`
  (`tests/phase7b_slice8.rs:65-68`), so a golden's inline source must NOT
  re-import `hosted::show` (a hard duplicate-qualifier error); a duplicate
  `import: intrinsics * ;` is harmless. This also means **every `(line N, col
  M)` in a paper-tests verbatim capture shifts by one line under the
  harness** — re-measure error goldens from the harness context; never
  transcribe the docs' standalone-fixture coordinates directly. Import
  spells for `core` consumers:
  `import: core::list | List Nil Cons | ;` +
  `import: core::iterator | Step Done More Iterator | ;`.
- Roadmap entry correction + a condensed-reference update land as a **docs
  commit at implementation time**, enumerated:
  - `docs/roadmap/P7b-higher-kinded-types.md:514-537` — the S6d entry still
    claims both `Slice[i64]`/`!Slice[i64]` get admitted and that the impl
    reads via `&!>` (both superseded: mutable closed by Ruling A, shared
    reads via `&>`); its `Size: S-M` line (`:537`) should read `M`; and its
    `:529-530` "admit … to the P7b.S8 lifted-mono route" mechanism claim is
    superseded by the sentinel patch (REQ-2), not the lift.
  - `docs/roadmap/ROADMAP.md:56` — the P7b row's "S6d — slices as Iterator
    targets, deferred behind those residuals" needs the same correction once
    this slice lands.
- Deferred register pointers to note on the tickets at implementation time:
  **SOO-60** (bound consumers; narrowed to the poly-body App-dispatch output
  loss), **SOO-45** (mutable target), **SOO-42** (narrowed by the probes to
  root-visible/inline consumers), **SOO-48** (generic-element targets,
  unreachable spelling), **SOO-41** (Ruling F multi-root `Deriv`, the paper
  tests' pointer for lifting the two-root refusal), **SOO-40** ("Poly
  signature audit: per-instantiation check for slice-bearing types (bundle
  exempt path)" — the instantiation-audit interface note above), **SOO-1**
  (close as superseded once G8 lands).
- Unit count: the phases below name **15** units (2+6+1+6), not the paper
  tests' rough "~14" — no unit was renamed or dropped, the estimate was just
  short by one.

## Phased delivery plan

P1–P4 are code-independent (parallelizable): none needs another's *code* to
land. Their automated exit goldens interlock pairwise, though — G13 (P1's
original exit golden) needs P2's sentinel to build, and G1 (P2's exit golden)
needs P1's per-impl inline keyword. Verified: `probes/s6d_a_fence_baseline.sth`
(G13) still fences at HEAD, and `probes/s6d_n_perimpl_inline_member.sth` (G1)
spells per-impl `: next inline`, needing REQ-3. P1's exit is therefore its
units plus the 6 canaries (P1-only, code-independent); P2's exit carries G13
as its build golden (the sentinel is what actually lifts the fence), plus the
P2-only pair G10/G11; G1 is evaluated once both have landed (whichever of
P1/P2 lands second, at latest by P5). P5 depends on all four. Every phase:
exit criteria are named G-goldens plus existing-suite-green (3480/0,
byte-identical bans, the 6 canaries and the PREREQ guard test); every changed
site gets its paper-test named units (15 total). Safety monotonicity holds at
every commit (NFR-3).

### P1 — Per-impl `inline` desugar (REQ-3)

- **Goal.** Impl members accept an optional `inline` keyword;
  `declares_inline` = impl spelling when present, else the trait member's.
- **Scope.** `src/parser.rs:4449-4453` (the inheritance point). Units:
  `impl_member_inline_keyword_sets_the_member_words_declares_inline`,
  `impl_member_without_inline_keyword_inherits_the_trait_member_flag`.
- **Out of bounds.** No trait-level change; the trait member's flag stays
  inherited when the impl is silent.
- **Entry.** Clean tree at HEAD.
- **Exit.** Both units pass; the 6 G22 List/Range canaries stay green
  (trait-level `inline` unchanged); suite green. **G13** itself cannot build
  from P1 alone — its fixture (`probes/s6d_a_fence_baseline.sth`) still hits
  the S2-6 fence until P2's sentinel lands, so G13 moves to P2's exit (below).
- **Parallelism.** Code-independent of P2/P3/P4. **Effort** S. **Difficulty**
  standard. **Blockers.** None (goldens G13/G1 are cross-phase; code is not).

### P2 — The two-half sentinel patch (REQ-2)

- **Goal.** `impl: Iterator for Slice[i64]` grounds its App-headed member row
  against the concrete slice target; the dispatch arm reads the grounded word.
- **Scope.** `src/parser.rs:855-964` (sentinel helpers), the slice branch
  inserted immediately before `is_mono_ctor_app` (`:4519`, preempting both
  `is_mono_ctor_app` and `is_concrete`),
  `src/ast.rs:2711` (invariant-exception doc + mutual pointer),
  `src/check/poly/ground.rs:1319-1333` (dispatch arm).
  Units: `rewrite_slice_sentinel_erases_every_sentinel_occurrence`,
  `slice_sentinel_is_recognizable_and_never_a_real_registry_index`,
  `slice_impl_member_grounds_app_headed_row_against_concrete_slice_target`,
  `slice_impl_member_non_var_free_grounding_reaches_the_standing_fences`,
  `slice_mono_member_call_reads_the_already_grounded_word_effect`,
  `non_slice_concrete_member_call_still_rederives_via_ground_member_type`.
- **Out of bounds.** No `PolyType::SliceApp` variant (fallback only); no
  `is_mono_ctor_app` change; the fence text stays byte-stable for non-slice
  targets.
- **Entry.** Clean tree at HEAD.
- **Exit.** **G13** builds (`3\n4\n`, now that the sentinel lifts the fence —
  P2's own build proof); **G10/G11** (the P1-free core: output ban holds,
  non-slice `Unit` target byte-identical); **G12** as a manual
  implementation-time spike (not a suite golden — see the success-criteria
  table): apply the dispatch arm alone (parser half only), run G8's fixture,
  capture the `ast.rs:2271` unreachable panic as evidence, then restore
  (`build_error_located`'s no-`panic` assertion, `tests/phase7b_slice8.rs:
  128-132`, would FAIL under a reverted arm, so it cannot be the golden that
  guards this — the spike is evidence collection, not a suite test); **G1**
  is conditional on P1 (needs the per-impl inline keyword) — if P2 lands
  first, defer G1's evaluation to whichever of P1/P2 lands second, at latest
  P5; suite green.
- **Parallelism.** Code-independent of P1/P3/P4 (see the phase-plan preamble
  for the golden-level interlock with P1: G13 needs P2, G1 needs P1). Re-run
  the growth-signal check (CLAUDE.md) against `src/parser.rs` at this exit —
  it now carries both P1's and P2's edits. **Effort** M. **Difficulty** hard
  (the invariant exception). **Blockers.** None on code; SliceApp fallback if
  the exception is rejected.

### P3 — The Step-monomorph clobber fix (REQ-4)

- **Goal.** Two monomorphs of one generated enum coexist in one program, each
  bare-name site resolving to its own.
- **Scope.** The env-hit candidate lookup at `src/check/terms.rs:966-968`:
  union it with `mint_fallback_candidates`'s check-time mints
  (`terms.rs:2249-2289`), keyed per-monomorph so each bare-name site still
  resolves to its own. `select_overload_fallback_sourced`
  (`builtins.rs:180-199`) is exonerated (operand-filters first, `:185-190`) —
  leave it alone. Unit:
  `variant_word_resolution_survives_a_second_monomorph_of_the_same_enum`.
- **Out of bounds.** Do not widen `select_overload_fallback_sourced`'s tier-1
  arm; do not touch lowering's bare-key map (`src/ir/func_builder/calls.rs:
  480` reads span-keyed first — no `src/ir/` change).
- **Entry.** Clean tree at HEAD (the bug is clean-tree reproducible, `s6d_m`).
- **Exit.** **G7** builds (`1\n2\n3\n`); suite green, including the two
  existing suite fixtures that already hold two monomorphs of one generated
  enum in one program —
  `tests/phase6_slice3b.rs:208`
  (`two_asymmetric_instantiations_eliminate_independently_in_one_word`, bare
  `Ok>`/`Err>` **destructures**) and
  `tests/phase7_slice12.rs:681`
  (`a_two_parameter_generic_enum_is_eliminated_at_swapped_monomorphs`, bare
  `Ok`/`Err` **constructor calls**, no destructure — the arms `drop` the
  payload) — named canaries, not just "nothing else moves".
- **Parallelism.** With P1/P2/P4. **Effort** M. **Difficulty** hard (resolution
  paths). **Blockers.** None.

### P4 — Borrow-join union + back-edge deriv forwarding (REQ-5)

- **Goal.** A rootless one-sided join asymmetry unions instead of refusing, at
  both join sites; `back_edge_outs` forwards `deriv`; rooted/two-root asymmetries
  stay refused.
- **Scope.** `src/check/terms.rs:3607-3620` (union), `src/check.rs:2718-2754`
  (eliminator merge), `src/check/terms.rs:3784` (back_edge deriv copy). Condition
  `owned_root.is_none()`. Units:
  `branch_join_unions_a_rootless_one_sided_asymmetry`,
  `branch_join_still_refuses_a_rooted_one_sided_asymmetry`,
  `branch_join_still_refuses_two_different_roots`,
  `eliminator_arm_merge_unions_a_rootless_one_sided_asymmetry`,
  `eliminator_arm_merge_still_refuses_a_rooted_asymmetry`,
  `back_edge_outs_forwards_deriv_along_the_index_map`.
- **Out of bounds.** No change to `check_reference_across_back_edge` (the
  argument-scan guard stays; G17 pins its reject). The PREREQ surviving-forward
  white-box test stays green.
- **Entry.** Clean tree at HEAD (the join rule is generic, no impl needed for
  G3/G4/G5).
- **Exit.** **G3** `build_ok`; **G4/G5** byte-identical refusals; the PREREQ
  guard test untouched; suite green. Re-run the growth-signal check
  (CLAUDE.md) against `src/check/terms.rs` at this exit — it now carries P3's
  clobber-fix edit plus P4's join/back-edge edits.
- **Parallelism.** With P1/P2/P3. **Effort** M. **Difficulty** hard (soundness
  argument). **Blockers.** None; F5 as-done fallback if a half stalls.

### P5 — The library impl + consumer goldens (REQ-6)

- **Goal.** Ship `impl: Iterator for Slice[i64]` in `lib/core/iterator.sth` with
  the natural inline body; the mono consumer goldens run; the bound-generic and
  times shapes stay closed.
- **Scope.** `lib/core/iterator.sth` (new impl). Goldens G1/G2/G6/G8/G14/G15/G16/
  G18 (build_run) and G9/G10/G17/G19/G20/G21 (build_error_located, byte-exact
  pinned from the patched binary).
- **Out of bounds.** No `for_each`/`fold` change (G20); no `!Slice` impl (G9);
  no `next`-in-quotation (G21).
- **Entry.** P1–P4 landed and green.
- **Exit.** **G8** (exit criterion, `1\n2\n3\n3\n3\n3\n`); **G1**
  (`3\n4\n`, unconditional here — both P1 and P2 have landed) and **G2**
  natural body (`3\n4\n`); **G6** while drain (`6\n6\n6\n`, if 5(b) landed);
  G15/G16/G18 mono drains; G9/G17/G19/G20/G21 byte-exact refusals; **G22**
  3480 + new / 0 with the 6 canaries green; the docs commit, enumerated: the
  `docs/roadmap/P7b-higher-kinded-types.md:514-537` S6d entry correction
  (the admissions line, the `&!>` read, and the `Size: S-M` line at `:537`
  → `M`, plus the `:529-530` lifted-mono-route mechanism claim superseded by
  the sentinel patch), `docs/roadmap/ROADMAP.md:56`'s matching correction,
  the condensed-reference update, and the ticket notes (SOO-42/45/60/48/41/40
  deferred pointers; SOO-1 closed as superseded now that G8 lands).
- **Parallelism.** None (depends on all). **Effort** M. **Difficulty** hard.
  **Blockers.** All of P1–P4.

Implementation-time discovery at the P5 exit: shipping the library impl made
two parse-time mints of one generated-enum header a permanent library fact
(the slice impl's `Step[i64 Slice[i64]]` beside the Range impl's), making
nullary ctor sites with two same-header candidates reachable — the one shape
the operand filter cannot discriminate. The shipped answer is a
strictly-narrowing consumer-type tie-break
(`generated_enum_consumer_type_pick`, `src/check/terms.rs`, the
`nullary_ctor_*` units), firing only where the operand-filtered set still
holds 2+ candidates of one generated-enum header AND a unique
`consumer_expected_type` match exists; no unique match keeps today's exact
Ambiguous bytes (unit-pinned). This deliberately retires the slice8b fence
golden per that fence's own doc prescription ("a later slice that grounds
nullary construction from its consuming context has to retire this
expectation deliberately"), renaming it
`two_instantiations_ground_a_bare_nullary_variant_from_its_consumer_type`.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Per-impl inline desugar (REQ-3): impl members accept an optional inline keyword at src/parser.rs:4449-4453; declares_inline = impl spelling when present else the trait member's inherited flag. Two units. Exit: both units pass and the 6 G22 List/Range canaries stay green with trait-level behaviour unchanged; suite green. G13 itself needs P2's sentinel to build, so it is evaluated as part of P2's exit, not P1's.",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "The two-half sentinel fence lift (REQ-2): parser half SLICE_SENTINEL_IDX/slice_sentinel/rewrite_slice_sentinel at src/parser.rs:855-964 plus the slice branch inserted immediately before is_mono_ctor_app at src/parser.rs:4519, preempting both is_mono_ctor_app and is_concrete; the PolyType::Generic invariant-exception doc at src/ast.rs:2711 with mutual pointers; and the dispatch half slice guard arm in resolve_mono_member_call at src/check/poly/ground.rs:1319-1333. Six units. Exit: G13 builds now that the sentinel lifts the fence, G10/G11 hold (the P1-free core), G12 is a manual implementation-time spike (not a suite golden). G1 is conditional on P1's inline keyword and is evaluated once both phases have landed. SliceApp is the recorded fallback if the invariant exception is rejected.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "The pre-existing Step-monomorph clobber fix (REQ-4): union the env-hit candidate arm at src/check/terms.rs:966-968 with mint_fallback_candidates's check-time mints (terms.rs:2249-2289), keyed per-monomorph, so two monomorphs of one generated enum coexist; select_overload_fallback_sourced (builtins.rs:180-199) is exonerated (operand-filters first) and must not be touched. One unit. Exit: G7 builds and the suite stays green, including the two pre-existing suite canaries that already carry two monomorphs of one generated enum -- tests/phase6_slice3b.rs:208 and tests/phase7_slice12.rs:681.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Borrow-join union + back_edge_outs deriv forwarding together (REQ-5): union a rootless one-sided asymmetry conditioned on owned_root.is_none() at BOTH join sites (check_branch_join src/check/terms.rs:3607-3620 and merge_arm_output_slot src/check.rs:2718-2754), and forward deriv alongside surviving in back_edge_outs at src/check/terms.rs:3784; rooted one-sided and two-root asymmetries stay refused byte-identically and check_reference_across_back_edge is untouched. Six units. Exit G3 build_ok, G4/G5 byte-identical, PREREQ guard test untouched.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "The library impl + consumer goldens (REQ-6): ship impl: Iterator for Slice[i64] with : next inline over len/&>/subslice/More/Done (natural body) in lib/core/iterator.sth; for_each/fold untouched. Exit criterion G8 (both impls drain through one imported protocol, 1/2/3/3/3/3); plus G1 (unconditional here, both P1 and P2 landed), G2/G6/G14/G15/G16/G18 build_run and G9/G10/G17/G19/G20/G21 byte-exact refusals; G22 suite green with the 6 canaries. Docs commit, enumerated: docs/roadmap/P7b-higher-kinded-types.md:514-537 (admissions line, &!> read, Size S-M->M at :537, and the :529-530 lifted-mono-route claim superseded by the sentinel patch), docs/roadmap/ROADMAP.md:56, the condensed-reference update, and the ticket notes (SOO-42/45/60/48/41/40 deferred, SOO-1 closed as superseded). Depends on phases 1-4.",
      "effort": "M",
      "difficulty": "hard"
    }
  ]
}
```
