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
  `rewrite_slice_sentinel` at `src/parser.rs:855-964`; the slice branch at
  `src/parser.rs:4629-4691` inside `parse_impl_member_body`, **preempting the
  `is_concrete` branch**. A fake `PolyType::Generic` carries the slice's element
  and is rewritten back to `Concrete(Type::Slice(..))` before any code reads its
  bogus header. This is a momentary, documented violation of
  `PolyType::Generic`'s own invariant: document the exception with **mutual
  pointers** at the construction site and at the invariant doc
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

Fix surface: generated-enum-word bare-name resolution — the S8b span-keyed
mechanism at `src/check/terms.rs:974-988`. The paper tests' verdict E
enumerates the six resolution paths and names the canaries; the risk is the
**other** paths (notably the fallback picker's tier-1 same-module preference,
`select_overload_fallback_sourced`, `src/check/builtins.rs:180-199`, verdict E
path 3). **Key the resolution per-monomorph; do not widen one arm.** Mechanism
design belongs to the implementer; the spec pins the invariant plus **G7/G8**.

### REQ-5 — Borrow-join union + `back_edge_outs` deriv forwarding, TOGETHER

- **Union at BOTH join sites** (A-amend 1): `check_branch_join`
  (`src/check/terms.rs:3607`, error at `:3613`) **and** `merge_arm_output_slot`
  (`src/check.rs:2718`, deriv match at `:2745`). Both carry the same refusal
  rule; patching only one leaves a `Step?`-dispatch-shaped state merge (the
  while drain's inner shape) refusing.
- **Condition: `owned_root.is_none()`** on the one-sided deriv (A-amend 2) — the
  same predicate the back-edge guard uses for its accept-case
  (`src/check.rs:1690-1700`) and `carried_borrow` for its no-contention case
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
arm (verdict C). Landing both closes that hole. **Load-bearing mechanism fact:**
naming a slice local always re-roots its deriv to a fresh **rootless** reborrow
(`src/check/terms.rs:248`; a parameter-seeded slice slot is `Slot::computed`,
`src/check/word_entry.rs:219-221`, so `held` is `None`), so rootless derivs are
ubiquitous in slice code and the union fires often — **G4/G5** are the
guardrails.

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
territory, unreached this slice). Mono consumers only: self-tail recursive drain
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
  every intermediate commit: **G4/G5/G9/G10/G11/G12/G17/G19/G20/G21** are the
  regression pins, plus the 6 List/Range canaries and the PREREQ guard test.
  Nothing admits a slice-bearing value into an escaping position at any step.

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
| G12 | negative control (ii): dispatch half | — | revert control + G8 call-site |
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

**In scope:** REQ-1..REQ-6 above and their goldens/units.

**Out of scope / non-goals (each with its note):**

- **No `src/ir/` change.** The roadmap's own S6d line said checker/target-grammar
  only; the delivered size is **M** — the roadmap entry needs a correction at
  implementation time (house convention: a docs commit lands with the phase
  work; see below).
- **`!Slice[i64]` mutable target** — Ruling A's sweep fires first (G9). SOO-45.
- **`Slice['T]` generic-element targets** — unreachable spelling (S6d-3), not a
  deferral.
- **`for_each`/`fold` and any bound-generic consumer** — closed at the `'It['T]`
  slot unification (G20). SOO-60 (the probes narrowed its scope to the poly-body
  App-dispatch output loss; worth noting on the ticket at implementation time).
- **The D-caveat:** a member call *inside a poly combinator splice* re-walks onto
  the S11 strict-grounding wall (`inline_combinator`,
  `src/check/poly/ground.rs:608-620`; `consumer_expected_type`'s spliced-body
  half declines for a mono member, `src/check/combinators.rs:549-556`). No golden
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

The five code changes are mutually independent except the lib impl, which
depends on all four. Recommended order mirrors the phase plan: land the parser
desugar (REQ-3) and the two-half sentinel (REQ-2) first (they unblock any live
exercise), the clobber fix (REQ-4) and the join/back-edge pair (REQ-5) in
parallel, then the lib impl + consumer goldens (REQ-6) last. Every changed site
gets the paper tests' named units (§Units, target ~14). Re-run the growth
signals (CLAUDE.md) at phase exit against every file this slice grew —
especially `src/check/terms.rs` (join sites + back_edge + clobber channel) and
`src/parser.rs` (sentinel helpers + desugar); split only if 2+ signals fire
together.

## Codebase map (path:line anchors verified against this tree)

Parser (REQ-2, REQ-3):

- `src/parser.rs:855-964` — `SLICE_SENTINEL_IDX`, `slice_sentinel`,
  `rewrite_slice_sentinel` (new helpers).
- `src/parser.rs:4629-4691` — the slice branch in `parse_impl_member_body`,
  preempting `is_concrete`.
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
- `src/ast.rs:2271` — the panic G12(a)'s revert control reaches if the dispatch
  arm is reverted.

Dispatch grounding (REQ-2 dispatch half, REQ-6 call sites):

- `src/check/poly/ground.rs:1319-1333` — the slice guard arm in
  `resolve_mono_member_call` (reads the already-grounded member word).
- `src/check/poly/ground.rs:1273-1442` — the mono member-call branch
  (sig-check, span-keyed record at `:1407`, `push_dispatch_outputs` at `:1441`);
  a mono call is a sig-check, not a body re-walk (verdict D).
- `src/check/poly/ground.rs:608-620` — `inline_combinator` (the D-caveat splice
  path; non-goal).

Join / back-edge (REQ-5):

- `src/check/terms.rs:3607-3620` — `check_branch_join`, error at `:3613`,
  `borrow_join_disagreement_error` at `:3797`. Union site 1.
- `src/check.rs:2718-2754` — `merge_arm_output_slot`, deriv match at `:2745`.
  Union site 2 (A-amend 1).
- `src/check.rs:1690-1700` — the back-edge guard's `owned_root: None` accept-case
  (the union's condition predicate).
- `src/check.rs:3164` — `carried_borrow`'s no-contention case (same predicate).
- `src/check/terms.rs:3773-3790` — `back_edge_outs`; copy `deriv` alongside
  `surviving` at `:3784`.
- `src/check/terms.rs:820-827` — the self-tail back-edge caller arm.
- `src/check.rs:1670` — `check_reference_across_back_edge` (the argument-scan
  guard; untouched, G16/G17 pin it).
- `src/check/terms.rs:248` — the name-read push reborrow arm (rootless mint).
- `src/check/word_entry.rs:219-221` — `Slot::computed` seeds a deriv-free
  parameter slot.
- `src/check/word_families.rs:252-256` — the exclusivity-scan predicate a kept
  deriv feeds.

Clobber fix (REQ-4):

- `src/check/terms.rs:974-988` — the S8b span-keyed generated-enum-word
  resolution (the named fix surface).
- `src/check/terms.rs:2249-2289` — `mint_fallback_candidates` (path 2).
- `src/check/builtins.rs:180-199` — `select_overload_fallback_sourced` (path 3,
  the tier-1 same-module preference; the risk).
- `src/check/terms.rs:1124-1157`, `:1210` — the span-keyed record arms.

Builtins / word families (reference — already work, PREREQ-shipped):

- `src/check/builtins.rs:520` — `Type::Slice(_, mutable, _) => !mutable` (Copy).
- `src/check/word_families.rs:32-63` — `&>`/`&!>`; `:842-861`, `:1020-1038` —
  `subslice`/`len` (mutability-preserving).

Library (REQ-6):

- `lib/core/iterator.sth` — add `impl: Iterator for Slice[i64]` with
  `: next inline ... ;` (natural body). `core::iterator` deliberately does not
  export bare `next`.

## Open questions and risks (adapted from paper tests §Risks and fallbacks)

- **F1 (shared only).** No risk: the mutable rejection is Ruling A's sweep,
  first-firing (G9); the `s6d_b3` twin catches a span/wording shift. SOO-45.
- **F2 (sentinel patch).** Negative controls cover it: (i) G11 byte-stability for
  non-slice targets, (ii) G12's revert control (the `ast.rs:2271` panic), (iii)
  G10 the output ban. If the invariant exception is ruled unacceptable, the
  fallback is `PolyType::SliceApp` — a bigger, unverified diff (S6d-2), not a
  spelling fallback.
- **F3 (per-impl inline).** Hard requirement, no spelling fallback. Trait-level
  inline is NOT acceptable (breaks the 6 canaries, G22). If the desugar stalls,
  the slice stalls: escalate.
- **F4 (clobber fix).** The gate leaves no other home for the impl (G19), so a
  stall ships nothing user-visible. Risk is the non-span-keyed paths (verdict E
  path 3); key the resolution, don't widen an arm. Pins: G7, G8.
- **F5 (join + back_edge, together).** Probe-proven fallback: the `s6d_a` as-done
  body with join/back_edge untouched (S6d-8.2/8.3). If 5(a) stalls: G2/G3 change
  to the as-done body, G4/G5 stay frozen. If 5(b) stalls: G6 is withdrawn (not
  the exit criterion). Trait-level inline is not a fallback here (F3).
- **F6 (lib impl).** for_each/fold stay out (G20). The natural body (G2) depends
  on 5(a) and verdict D's mono-caller finding; the as-done body is the spelling
  fallback if the S11-splice caveat ever matters.

## Housekeeping

- Probe fixtures are already committed (`40aacce`, `e1fa5c8`) — implementation
  reuses them as-is, does not re-derive.
- Goldens land in the `tests/phase7b_slice8.rs` successor convention (look at how
  slice goldens are organized there; keep `single_file_hosted` /
  `build_ok` / `build_run_keep` / `build_error_located`). Harness note: the
  harness prepends `import: intrinsics * ;` + `import: hosted::show | . | ;`, so a
  golden's inline source must NOT re-import `hosted::show` (a duplicate collides
  in the seen-map); the standalone probe fixture keeps its own import. Import
  spells for `core` consumers:
  `import: core::list | List Nil Cons | ;` +
  `import: core::iterator | Step Done More Iterator | ;`.
- Roadmap entry correction + a condensed-reference update land as a **docs
  commit at implementation time**: the roadmap's S6d entry (~line 514) still
  claims both `Slice[i64]`/`!Slice[i64]` get admitted and that the impl reads via
  `&!>` — both superseded (mutable closed by Ruling A; shared reads via `&>`),
  and its `S-M` line should read `M`.
- Deferred register pointers to note on the tickets at implementation time:
  **SOO-60** (bound consumers; narrowed to the poly-body App-dispatch output
  loss), **SOO-45** (mutable target), **SOO-42** (narrowed by the probes to
  root-visible/inline consumers).

## Phased delivery plan

P1–P4 are mutually independent (parallelizable). P5 depends on all four.
Every phase: exit criteria are named G-goldens plus existing-suite-green
(3480/0, byte-identical bans, the 6 canaries and the PREREQ guard test); every
changed site gets its paper-test named units. Safety monotonicity holds at every
commit (NFR-3).

### P1 — Per-impl `inline` desugar (REQ-3)

- **Goal.** Impl members accept an optional `inline` keyword;
  `declares_inline` = impl spelling when present, else the trait member's.
- **Scope.** `src/parser.rs:4449-4453` (the inheritance point). Units:
  `impl_member_inline_keyword_sets_the_member_words_declares_inline`,
  `impl_member_without_inline_keyword_inherits_the_trait_member_flag`.
- **Out of bounds.** No trait-level change; the trait member's flag stays
  inherited when the impl is silent (G13 pins the old spelling still builds).
- **Entry.** Clean tree at HEAD.
- **Exit.** **G13** builds (`3\n4\n`); the 6 G22 canaries green with List/Range
  unchanged; units pass.
- **Parallelism.** With P2/P3/P4. **Effort** S. **Difficulty** standard.
  **Blockers.** None.

### P2 — The two-half sentinel patch (REQ-2)

- **Goal.** `impl: Iterator for Slice[i64]` grounds its App-headed member row
  against the concrete slice target; the dispatch arm reads the grounded word.
- **Scope.** `src/parser.rs:855-964` (sentinel helpers), `:4629-4691` (the
  slice branch preempting `is_concrete`), `src/ast.rs:2711` (invariant-exception
  doc + mutual pointer), `src/check/poly/ground.rs:1319-1333` (dispatch arm).
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
- **Exit.** **G1** builds (`3\n4\n`, as-done body isolating the sentinel+inline
  from REQ-5); **G11** byte-identical (non-slice `Unit` target still fences);
  **G10** the output ban holds; **G12** revert control (dispatch arm reverted →
  the `ast.rs:2271` panic, guarded by `build_error_located`'s no-`panic`
  discipline); suite green.
- **Parallelism.** With P1/P3/P4 (P2 needs P1 only for the *inline* golden G1;
  sequence P1→P2 if run serially, else the shared branch merges cleanly).
  **Effort** M. **Difficulty** hard (the invariant exception). **Blockers.**
  None; SliceApp fallback if the exception is rejected.

### P3 — The Step-monomorph clobber fix (REQ-4)

- **Goal.** Two monomorphs of one generated enum coexist in one program, each
  bare-name site resolving to its own.
- **Scope.** The S8b span-keyed channel at `src/check/terms.rs:974-988` (key the
  resolution per-monomorph; mechanism is the implementer's). Beware paths 2/3
  (`mint_fallback_candidates` `terms.rs:2249-2289`,
  `select_overload_fallback_sourced` `builtins.rs:180-199`). Unit:
  `variant_word_resolution_survives_a_second_monomorph_of_the_same_enum`.
- **Out of bounds.** Do not widen a single fallback arm; do not touch lowering's
  bare-key map (`src/ir/func_builder/calls.rs:480` reads span-keyed first — no
  `src/ir/` change).
- **Entry.** Clean tree at HEAD (the bug is clean-tree reproducible, `s6d_m`).
- **Exit.** **G7** builds (`1\n2\n3\n`); suite green (no existing fixture has two
  monomorphs of one generated enum, verdict E, so nothing else moves).
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
  guard test untouched; suite green.
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
- **Exit.** **G8** (exit criterion, `1\n2\n3\n3\n3\n3\n`); **G2** natural body
  (`3\n4\n`); **G6** while drain (`6\n6\n6\n`, if 5(b) landed); G15/G16/G18 mono
  drains; G9/G17/G19/G20/G21 byte-exact refusals; **G22** 3480 + new / 0 with the
  6 canaries green; the docs commit (roadmap correction + condensed reference).
- **Parallelism.** None (depends on all). **Effort** M. **Difficulty** hard.
  **Blockers.** All of P1–P4.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Per-impl inline desugar (REQ-3): impl members accept an optional inline keyword at src/parser.rs:4449-4453; declares_inline = impl spelling when present else the trait member's inherited flag. Two units. Exit G13 builds and the 6 G22 List/Range canaries stay green with trait-level behaviour unchanged.",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "The two-half sentinel fence lift (REQ-2): parser half SLICE_SENTINEL_IDX/slice_sentinel/rewrite_slice_sentinel at src/parser.rs:855-964 plus the slice branch at :4629-4691 preempting is_concrete, the PolyType::Generic invariant-exception doc at src/ast.rs:2711 with mutual pointers, and the dispatch half slice guard arm in resolve_mono_member_call at src/check/poly/ground.rs:1319-1333. Six units. Exit G1 builds, G11 byte-identical for a non-slice concrete target, G10 output ban holds, G12 revert control. SliceApp is the recorded fallback if the invariant exception is rejected.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "The pre-existing Step-monomorph clobber fix (REQ-4): key the S8b span-keyed generated-enum-word resolution at src/check/terms.rs:974-988 per-monomorph so two monomorphs of one enum coexist, watching mint_fallback_candidates (terms.rs:2249-2289) and select_overload_fallback_sourced's tier-1 preference (builtins.rs:180-199); do not widen a single arm and do not touch src/ir. One unit. Exit G7 builds and the suite stays green.",
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
      "focus": "The library impl + consumer goldens (REQ-6): ship impl: Iterator for Slice[i64] with : next inline over len/&>/subslice/More/Done (natural body) in lib/core/iterator.sth; for_each/fold untouched. Exit criterion G8 (both impls drain through one imported protocol, 1/2/3/3/3/3); plus G2/G6/G14/G15/G16/G18 build_run and G9/G10/G17/G19/G20/G21 byte-exact refusals; G22 suite green with the 6 canaries. Docs commit: roadmap S6d entry correction (mutable closed by Ruling A, shared reads via &>, size M) and the condensed reference. Depends on phases 1-4.",
      "effort": "M",
      "difficulty": "hard"
    }
  ]
}
```
