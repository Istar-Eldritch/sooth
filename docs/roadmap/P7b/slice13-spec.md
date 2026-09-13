# P7b.S13 spec — cross-call App lift (SOO-60)

**Status: planned.** Base `a3779f4` (worktree `soo-60`), suite 3528 / 0
(re-verified this round). Discovery is ground truth:
[slice13-probes](./slice13-probes.md) (today's bytes, reachability, moved-pin
inventory; frozen baseline `probes/s13_baseline.md`) and
[slice13-paper-tests](./slice13-paper-tests.md) (validated goldens G1–G11, the
mechanism walkthrough, the three recorded rulings; G12 below is this round's
user-ratified addition). This file adopts that
mechanism and those goldens as its approach and acceptance criteria. Format
precedent: [slice12-spec](./slice12-spec.md), [slice12-brief](./slice12-brief.md)
(R4 recorded the deferral this slice lifts; PB-5 open-questions pattern).

## Problem

Two located fences in `poly_cross_match` / `poly_cross_output`
(`src/check/poly/crosscall.rs`) reject poly→poly cross-calls over
higher-kinded shapes:

- **The input fence (S1-17.i, `crosscall.rs:346-353`).** The two-sided pattern
  `(PolyType::App { .. }, _) | (_, PolyType::App { .. })` (comment :344-345,
  message :351), sitting ahead of the catch-all mismatch at :354, rejects any
  `App` slot on either side of an INPUT match: "a higher-kinded application in
  a cross-called polymorphic word is not yet supported from a polymorphic body"
  (round A/B1/D1–D4/E bytes).
- **The compound-output wildcard (`poly_cross_output`, `crosscall.rs:398-405`,
  message :403).** Rejects every compound OUTPUT the same way: "returning the
  compound type `…` from a polymorphic word is not yet supported from a
  polymorphic body" (round C2/C4 bytes).

This is the remaining facet of the roadmap sentence "poly bodies calling poly
words over compound receivers". S12 (SOO-39) fixed the member-dispatch facet;
poly-word self-calls were always fine (structural pointwise match). SOO-60
lifts both fences so an App-headed cross-call checks, grounds, and lowers
through the same machinery the mono route already uses.

The hard part (confirmed in discovery) is the interaction with the R6 growth
ban: an App operand facing a **bare-var** declared input is the GROWTH error,
not the fence, because the `(PolyType::Var(v), _)` arm (`crosscall.rs:208`)
precedes the App arm (round B2). App/Generic output rendering needs **no**
registry interning at walk time (symbolic; interning happens at grounding) —
a claim about *interning* (registry writes/mints) only: the R-13.1 head bind
still READS the generics registry to mint the ctor image (REQ-1.2).
The R4 fence holds: lifting must not disturb S12's render fix.

## Requirements

Line numbers measured at `a3779f4` (paper-tests fact 8; the handed-down
d7559bb numbers are one line stale).

**REQ-1 — lift the input fence with guarded declared-App arms.** Replace the
S1-17.i fence arm (`crosscall.rs:346-353`, comment :344-345) with two guarded
arms, `(App{..}, App{..})` and `(App{..}, Generic{..})`, each carrying its
arity test as a match guard so a guard failure reaches the unchanged
`_ => Err(mismatch())` catch-all at :354 — the same mechanism the
same-header Generic/Generic guard's failure already takes
(`crosscall.rs:321-343`). **Pinned shape: the guarded arms.** An arm body
cannot "fall through", and the alternative — an inline `Err(mismatch())`
for the unmatched sub-shapes — is a byte-identical outcome; this spec pins
one so the implementation does not choose. The sub-dispatch on the
`supplied` type is:

1. `App { .. }` (guarded arm, arity guard `declared args len == supplied
   args len`): bind the callee's head variable through the SAME
   consistency/conflict block the Var arm uses (`crosscall.rs:283-296`: find
   previous image, `poly_cross_var_conflict_error` on disagreement, else
   push), then recurse on args pairwise via `poly_cross_match`. An
   arity-mismatching guard failure rides the guard to :354 → `mismatch()`.
   Witnessed by D1/D2/E (G3/G4/G1).
2. `Generic { .. }` (guarded arm, same arity guard): the R-13.1 ruling — bind
   the head to a ctor-valued image
   `Image::Concrete(ctor_image_type(&generics, gid-of-the-supplied-header))`,
   same arg recursion. `ctx.generics()` returns
   `Option<&RefCell<GenericTypes>>` (`engine.rs:1455`) and `ctor_image_type`
   takes `&GenericTypes` (`ast.rs:3596`), so the arm borrows the cell —
   `Some(cell) => cell.borrow()` — and the `None` arm reuses
   `poly_generic_not_yet_groundable_error` (the mono precedent,
   `unify.rs:425-432`), keeping the no-new-diagnostic-text posture. The head
   bind READS the generics registry (`ctor_image_type` reads the ctor's
   declared name off it); "no registry interning" (REQ-4's walk-time claim)
   is about interning only. A supplied Generic carrying `len_args` takes the
   mono route's S1-7 rejection (`poly_app_len_domain_unsupported_error`;
   precedent `unify.rs:467-477`) — checked INSIDE the arm body, because it
   is a distinct diagnostic: as a match guard it could only reach :354's
   mismatch. Witnessed by D4 (G6).
3. `Var(_)` (a supplied bare variable): no guarded arm matches, so it
   reaches the :354 catch-all — `mismatch()`. Witnessed by D3 (G5).
4. everything else (Concrete/Array/Ref/OwnedCell/QuotLit/Quotation/
   GenericVariant, and any arity-failing App/Generic): the same :354
   catch-all `mismatch()` — byte-identical to today's verdicts for those
   shapes.

Load-bearing for the phase-1 units: the planned direct-call unit harness
`probe_ctx` (`src/check/poly/tests.rs:1694-1696`) passes `generics: None`,
so a direct-call (App, Generic) unit as specified exercises the `None`
verdict (`poly_generic_not_yet_groundable_error`), not the mint — units
that mint or read the registry (the ctor-image bind, the `len_args`
rejection) must build a Ctx carrying generics; the phase-1 unit list says
which.

**REQ-2 — delete the second fence pattern (R-13.2).** The `(_, PolyType::App
{ .. })` half is removed: a supplied App against a declared non-App slot
becomes a mismatch — except a declared bare Var, where the Var arm
(`crosscall.rs:208`) precedes everything and keeps GROWTH. No pin exists on
this face (round G inventory has none), so the deletion is deliberate, not an
oversight.

**REQ-3 — growth ban untouched.** The `(PolyType::Var(v), _)` arm at
`crosscall.rs:208` and its supplied-match (:210-232) are not edited; they
precede the new App arm, so an App operand facing a bare-var declared input
stays the growth error (round B2; unit pins tests.rs:3755,
phase7_slice3k.rs:195/:208). This is probe fact 1 and the B2 precedence: if G2
moves, the slice has broken precedence.

**REQ-4 — add the output arms.** In `poly_cross_output`
(`crosscall.rs:365-405`), before the wildcard, add `Generic` (C2/G7), `Array`
(C4/G8), and `App` (G9) arms, each recursing per variable through the existing
Var-arm mapping lookup (:375-391: `Image::Concrete(t)` → `Concrete(t)`,
`Image::CallerVar(w)` → `Var(w)`). `len_args` and array lengths pass through
concrete — the App arm's rebuilt Generic carries `len_args: vec![]`, sound
because S1-7 fences len-domain headers upstream (the signature gate's
length-variable fence, `crosscall.rs:443-445`, and REQ-1.2's in-body
rejection); `Len::Var` is fenced upstream by `poly_cross_signature_supported`
("a length variable in the callee's signature", `crosscall.rs:443-445`). The
`App` arm's head renders `Image::CallerVar(w)` → `PolyType::Var(w)` (D1/E) and
`Image::Concrete(CtorImage(gid, name))` → `PolyType::Generic{gid's header,
mapped args}` (D4); a non-CtorImage `Concrete` head is unreachable (round F)
and guards to `mismatch()`. **Ref gains no arm** (C3: Ref outputs are banned at
declaration, "a reference cannot be stored", both spellings — the wildcard's
Ref face is unreachable and stays as defense). **GenericVariant stays in the
wildcard** (the R3.4/R3.5 convention comment above it, :392-397).

**REQ-5 — no new grounding/lowering machinery.** Grounding is existing code:
compose (`instantiate.rs:682`) folds the mapping into θ_h
(`Image::Concrete(t)` → `*t` at :696-698, a CtorImage head landing in
`subst.ty` exactly as the mono route leaves it; `CallerVar(u)` →
`caller.subst.ty_of(u)` at :700-702), and `apply_subst` (`unify.rs:598-607`)
grounds the callee's outputs — its registries intern
Generic/Array/Ref/cell shapes (`intern_array_type` :657, `intern_ref_type`
:693, `intern_owned_cell_type` :700) and mint Generic/GenericVariant/App via
`ctx.generics()` (the Generic mint calls at `unify.rs:756`/`:758`;
GenericVariant :808-816; App :903-909; `Ctx::generics()`
`engine.rs:1455`). Lowering asserts the App-head-CtorImage invariant
(`src/ir/driver.rs:708-711`; `src/driver.rs` is a different file with no
`CtorImage`). `is_copy(CtorImage)` = false (`src/check/builtins.rs:560`)
keeps the walk-time `Bound::Copy` discharge honest. The diagnostic side
needs no new renderer either: `poly_image_str` (`crosscall.rs:480-485`)
renders a concrete image through `Type::name`, which for a `CtorImage`
returns the bare ctor name (`src/ast.rs:3571`) — diagnostic-only: two
same-named ctors from different modules render identically in
`poly_cross_var_conflict_error` text; accepted for this slice (no
unsoundness), noted as a follow-up candidate. No new `PolyType` variant, no
new unification rule.

**REQ-6 — the ctor-image head rides `Image::Concrete(Type::CtorImage)`.** No
new `Image` variant (`ast.rs:3007-3010`); `ctor_image_type` (`ast.rs:3596`)
stays the sole constructor. The Image doc comment ("so no type constructor ever
needs representing here", `ast.rs:2999-3005`) is stale after the lift and must
be rewritten — **comment-only, no ast.rs behavior change**.

## Acceptance criteria — the G1–G12 goldens

Fixture texts, today/post-fix expectations, and mechanism walkthroughs live in
[slice13-paper-tests](./slice13-paper-tests.md); harness conventions
(`single_file_hosted`, `build_run_keep`, `build_ok`, `build_error_located`)
from `tests/phase7b_slice8.rs`, plus a `build_error_bare` copy (precedent
`tests/phase7b_slice12.rs:152`: byte-verbatim source in a bare temp dir, so
line numbers match a frozen baseline) carried into `tests/phase7b_slice13.rs`
for G2. Phase 1 creates `tests/phase7b_slice13.rs` (with the
`build_error_bare` copy) — G2 and G5 are its first goldens. Harness copies
drop the fixture's own `import: intrinsics * ;` (a duplicate collides in
the import seen-map — collision errors at `declarations.rs:906`/`:912-918`)
— which shifts a diagnostic
one line down (6→7), so a baseline byte-pin cannot ride
`build_error_located`. Goldens land in `tests/phase7b_slice13.rs`.

| Golden | Pins | Harness |
| --- | --- | --- |
| G1 `app_pass_through_cross_call_builds_and_runs_clean` | `probes/s13_e_post_lift_green.sth` verbatim: App pass-through cross-call, exit 0, stdout empty | `build_run_keep` |
| G2 `bare_var_supplied_app_operand_still_grows_byte_identically` | `probes/s13_b_bare_var_growth.sth` verbatim: growth bytes byte-identical (R-13.3/REQ-3 must-not-move) | `build_error_bare` copy + baseline byte-pin (`build_error_located` shifts line 6→7 and cannot pin) |
| G3 `app_vs_app_head_and_arg_vars_both_bind` | edited D1: App-vs-App, arg vars differ; exit 0 | `build_run_keep` |
| G4 `app_vs_app_concrete_arg_grounds_the_element` | edited D2: concrete arg `'It[i64]`; exit 0 | `build_run_keep` |
| G5 `bare_var_supplied_where_app_declared_is_a_rendered_mismatch` | `probes/s13_d_bare_supplied.sth` verbatim: verdict MOVES fence→rendered mismatch; **exact bytes pinned at implementation time from the live binary** | `build_error_located` |
| G6 `concrete_ctor_supplied_as_head_binds_the_ctor_image` | edited D4 (R-13.1): ctor-image head bind end-to-end; exit 0 | `build_run_keep` |
| G7 `generic_output_renders_through_the_mapping` | edited C2 (aligned spelling): Generic output through the mapping; exit 0 | `build_run_keep` |
| G8 `array_output_renders_through_the_mapping` | edited C4: Array output through the mapping (walk-time render; no intrinsics-only array ctor) | `build_ok` |
| G9 `the_e_shape_exercises_both_lifted_arms_end_to_end` | G1's fixture, mechanism walkthrough (input arm step 2, output arm step 3, compose/apply_subst steps 4–5) — no extra run | (covered by G1) |
| G10 `moved_pins_retarget` | the four moving pins (below) | mixed |
| G11 `existing_suite_green` | full suite green (3528 + this slice's tests, 0 failed) | `cargo test` |
| G12 `cursor_bounded_app_pass_through_grounds_end_to_end` | the G10.4 source (the Cursor-bounded App pass-through pair pinned at `tests/phase7b_slice12.rs:450`) plus a spelled grounding main that instantiates the bounded consumer end-to-end — exercises compose's `resolve_user_bound` with `ty = CtorImage` (R-13.3's path); fixture text and bytes finalized and pinned at implementation time from the live binary (the grounding main's typing is unverified at design time); **required by user ruling this round** | `build_run_keep`, exit 0, no stdout (pin the harness choice at implementation if the grounding main's typing forces an adjustment) |

**G5/G10.2 bytes** are pinned at implementation time from the live binary per
house convention (the only verdicts that MOVE; the `note: declared` tail
renders the caller's effect via `effect_str`). **G12's fixture** is finalized
at implementation time the same way: its grounding main's typing is
unverified at design time, so the text and its bytes are pinned from the
live binary then (assertion intent in the table; pin the harness choice too
if the typing forces an adjustment).

**Moved-pin retargets (G10, round G inventory):**

1. `src/check/poly/tests.rs:608`
   `non_member_app_cross_call_still_rejects_with_p8_fence_text` — post-lift the
   shape checks clean (input (App, App) binds `G→CallerVar(G)`,
   `T→CallerVar(T)`; no outputs, no bounds). Flip to a green assertion, rename
   e.g. `non_member_app_cross_call_checks_clean_after_the_lift`, delete the two
   fence-text asserts (:617, :622), rewrite the stale doc comment (:602-606).
2. `src/check/poly/tests.rs:1782`
   `poly_cross_match_app_slot_is_unsupported_not_a_panic` — direct
   `poly_cross_match(App{head:0, args:[Var(1)]}, Var(0))`, exactly G5's shape.
   Retarget to expect the mismatch (assert "type mismatch" + the two
   renderings `'F['T]` expected, `'F` found), rename e.g.
   `poly_cross_match_app_slot_vs_bare_var_is_a_rendered_mismatch`.
3. `src/check/poly/tests.rs:3935`
   `check_cross_call_unsupported_callee_shapes_name_themselves`, sub-fixture 2
   (:3942-3947, "returning the compound type `Box['U]` …") — post-lift green
   (Generic output renders through the mapping, `drop` consumes it).
   RETARGET IN PLACE (phase 2): keep the tuple, flip its expectation to the
   post-lift clean check (same source, green), rename/comment accordingly —
   it is the poly-body-drop twin of G7 (`: g ( 'T -- ) box drop ;` is a shape
   no other golden covers). Sub-fixtures 1 (:3940, length var) and 3
   (:3948-3952, row-polymorphic) are different fences and stay.
4. `tests/phase7b_slice12.rs:450` `cross_call_app_fence_stays_byte_identical`
   (byte `assert_eq!` :467-470, expected string :469) — S12's G8. Post-lift the
   Cursor-bounded pass-through pair checks clean (`Bound::User(Cursor)`
   discharges symbolically, `crosscall.rs:56`). Replace the byte-pin with a
   green assertion (same source + `: main ( -- ) ;` through `build_ok`), rename
   e.g. `cross_call_app_slot_checks_clean_after_the_hkt_lift`, rewrite the doc
   comment (:441-448) to record SOO-60 as the lifting slice. The grounding twin
   is REQUIRED golden G12 (phase 3), not optional follow-up.

**Full gate:** `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Phased delivery

Each phase exits green independently (`cargo fmt --check && cargo clippy -- -D
warnings && cargo test`); a phase is not done until its goldens pass and its
new stage code has unit coverage.

### Phase 1 — the input arm

- **Entry:** clean tree at `a3779f4`, suite 3528 / 0.
- **Do:** REQ-1 (the guarded declared-App arms), REQ-2 (delete the second
  fence pattern), REQ-3 (leave the Var arm untouched) in
  `src/check/poly/crosscall.rs`. Retarget the two unit pins: tests.rs:608 →
  green Ok (a check_src-level flip; the stale doc comment :602-606
  rewritten) and tests.rs:1782 → the G5 mismatch. Retarget the S12 byte-pin:
  phase7b_slice12.rs:450 `cross_call_app_fence_stays_byte_identical` → green
  Ok — its fixture is input-fence-only (inputs (App,App) with concrete
  args, output i64 concrete), so phase 1's input arm alone flips it, and
  the concrete output never touches `poly_cross_output`'s wildcard. Same
  source + `: main ( -- ) ;` through `build_ok`; rename e.g.
  `cross_call_app_slot_checks_clean_after_the_hkt_lift`; rewrite the stale
  doc comment (:441-448). Add units beside the arm: App-vs-App head+arg
  bind, concrete-arg bind, (App, Var) → mismatch, (App, Generic) →
  ctor-image bind (builds a Ctx carrying generics — `probe_ctx` passes
  `generics: None`, REQ-1's note), supplied Generic carrying `len_args` →
  `poly_app_len_domain_unsupported_error` (also on a generics-carrying
  Ctx), arity-mismatch → mismatch, head-var conflict →
  `poly_cross_var_conflict_error`, the R-13.2 supplied-App-vs-non-App face.
- **Files:** `src/check/poly/crosscall.rs` (:208 precedence preserved,
  :283-296 reused, the fence block rewritten — comment :344-345, arm
  :346-353 — with the :354 catch-all kept), `src/check/poly/tests.rs` (:608,
  :1782), `tests/phase7b_slice12.rs` (:441-448, :450, :467-470),
  `tests/phase7b_slice13.rs` (new — created this phase with the
  `build_error_bare` copy; G2 and G5 are its first goldens).
- **Exit:** G2 (growth byte-identical), G5 (rendered mismatch, bytes pinned
  live), the G10.4 retarget (slice12 byte-pin → green Ok) and the
  tests.rs:608 flip green; new arm has unit coverage; full gate green.

### Phase 2 — the output arms

- **Entry:** phase 1 green.
- **Do:** REQ-4 (Generic/Array/App arms in `poly_cross_output`,
  `crosscall.rs:365-405`; Ref no arm, GenericVariant stays wildcarded). REQ-6
  ast.rs comment rewrite (`ast.rs:2999-3005`, comment-only). Retarget
  tests.rs:3935 sub-fixture 2 (:3942-3947) IN PLACE: keep the tuple, flip
  its expectation to the post-lift clean check (same source, green), rename
  and re-comment — this phase's Generic output arm is what flips it, and it
  is the poly-body-drop twin of G7. Units beside the arms (in
  `src/check/poly/tests.rs` — `crosscall.rs` has no `#[cfg(test)]` module):
  Generic output through the mapping, Array output through the mapping,
  App output head `CallerVar`→`Var` and `CtorImage`→`Generic`, non-CtorImage
  Concrete head → mismatch, `len_args`/length pass-through. Add goldens
  G1/G3/G4/G6/G7/G8/G9 in `tests/phase7b_slice13.rs`.
- **Files:** `src/check/poly/crosscall.rs`, `src/check/poly/tests.rs`
  (:3935 table — sub-fixture 2 :3942-3947 retargeted in place), `src/ast.rs`
  (comment only), `tests/phase7b_slice13.rs` (extends the file phase 1
  created), edited fixture copies
  inline in the harness (no repo fixtures added).
- **Exit:** G1/G3/G4/G6/G7/G8/G9 green (grounding drives the end-to-end chain
  via G1/G6); the sub-fixture-2 retarget green; output arms have unit
  coverage; full gate green.

### Phase 3 — golden G12, docs, growth-structure re-check

- **Entry:** phases 1–2 green.
- **Do:** add the required golden G12 (user ruling this round) in
  `tests/phase7b_slice13.rs`: the G10.4 source (the Cursor-bounded App
  pass-through pair from phase7b_slice12.rs:450's fixture) plus a spelled
  grounding main that instantiates the bounded consumer end-to-end — the
  golden that exercises compose's `resolve_user_bound` with
  `ty = CtorImage` (R-13.3's path). Its fixture text is finalized and its
  bytes pinned at implementation time from the live binary (the grounding
  main's typing is unverified at design time); assertion intent
  `build_run_keep`, exit 0, no stdout. Update the docs, worded to what
  exists: amend the aggregate P7b row in `docs/roadmap/ROADMAP.md` (one
  aggregate row at :56, no per-slice rows; the `S13` ruling there is P7's
  poly.rs slice, so name this slice **P7b.S13** everywhere) and add the
  lift statement to `docs/roadmap/P7b-higher-kinded-types.md` (it carries
  no existing S1-17 / cross-call fence prose to edit). Re-run the
  growth-structure signals on `crosscall.rs` as it now stands and record
  the verdict. G10 sweep: all four retargets must be green by now (nothing
  remains in this phase).
- **Files:** `tests/phase7b_slice13.rs` (G12),
  `docs/roadmap/P7b-higher-kinded-types.md`, `docs/roadmap/ROADMAP.md`.
- **Exit:** G12 green; G10 (all four pins retargeted across phases 1–2),
  G11 (suite green) satisfied; the R6/growth, concrete-compound, quotation,
  inline-routing, and ambiguity pins stay green (paper-tests G11 canaries);
  full gate green.

## Out of bounds

- **No S6d work** — no sentinel grounding, no slice impl, no `PolySlot`
  provenance (`Deriv`).
- **No S12 render changes** — every `tests/phase7b_slice12.rs` golden other
  than the G10.4 retarget stays byte-identical (R4 fence).
- **No diagnostic text changes** beyond the two moving verdicts (G5, G10.2).
  No third message on the R-13.2 face (that would be a new diagnostic, out of
  the no-new-text posture).
- **No new `Image` or `PolyType` variant** — the ctor-image head rides
  `Image::Concrete(Type::CtorImage)`.
- **No growth-verdict change** — the Var arm and its supplied-match are not
  edited.

## Risks

Distinct from the open questions below; what could bite at implementation
time.

- **Bound discharge over a CtorImage is unverified at design time.**
  Compose's `resolve_user_bound` with `ty = CtorImage` (R-13.3) had no
  golden when this spec was written; G12 (phase 3) now covers it, but its
  fixture typing is unverified until implemented — a defect there is an
  implementation-time P0 (fix or scope explicitly).
- **`ctx.generics()` is absent on standalone probe Ctxs.** `probe_ctx`
  (`src/check/poly/tests.rs:1694-1696`) passes `generics: None`, so
  direct-call units over the mint/`len_args` faces hit the
  not-yet-groundable verdict unless the unit builds a Ctx carrying
  generics (the phase-1 unit list says which).
- **The R-13.2 face is unpinned.** Supplied App vs declared non-App is
  deliberately deleted to the :354 catch-all with no fixture; a regression
  there would be silent until a pin exists.
- **Post-lift green predictions are unvalidated by execution.** Every
  "post-fix: green" claim in the acceptance table is a paper prediction;
  none has run until implementation. The goldens are the discovery
  mechanism, not a record of observed runs.

## Open questions — recorded rulings awaiting ratification (PB-5)

- **R-13.1 (the D4 ctor-image head bind — the slice's one real design bit).**
  CHOSEN: bind the callee's head variable to `Image::Concrete(Type::CtorImage)`
  minted via `ctor_image_type` over the supplied Generic's header. REJECTED:
  the rendered mismatch ("you cannot pass a concrete ctor where a higher-kinded
  variable is declared"). Consequence of the rejection: the cross-call route
  becomes strictly weaker than the mono route for the identical shape (a mono
  call site grounds `'It:=Wrap` over an App-declared word clean today, round E
  incidental, `unify.rs:424-513`), and D4's twin spelled `'It['T]` (the E
  shape) would go green while the `Wrap['T]` spelling stayed red with no
  kind-level difference. The chosen ruling reuses established App-head
  semantics end to end: bind (`unify.rs:488-497`), ground
  (`unify.rs:843-922`), lower (`src/ir/driver.rs:708-711`), symbol-key on `GenericId`
  (`ast.rs:3055-3060`). **Override path:** if the spec is ratified to the
  rejection, only G6 flips to `build_error_located` with mismatch bytes pinned
  live; G10.4's retarget is unchanged; nothing else moves.
- **R-13.2 (supplied App vs declared non-App face).** Post-lift: rendered
  mismatch via the :354 catch-all (the deleted second fence pattern hands the
  face there). No fixture or pin exercises it (round G inventory has none), so
  the deletion is deliberate, recorded so it is not read as an oversight. If
  ratified to keep a named rejection there, it is a new (third) diagnostic and
  out of this slice's no-new-text posture. **Unpinned today.**
- **R-13.3 (bound discharge over a ctor-image head — user-ratified this
  round).** The lift adds no machinery: the walk-time `Bound::Copy` arm
  degrades honestly (`is_copy(CtorImage)` = false,
  `src/check/builtins.rs:560`, copy-bound rejection fires,
  `crosscall.rs:87-88`); a `Bound::User` on a ctor-image head defers to
  compose's `resolve_user_bound` loop (`crosscall.rs:104` defers,
  `instantiate.rs:726-750` resolves) with `ty = CtorImage`. Golden G12
  (required, phase 3) now VERIFIES that existing path end to end: the G10.4
  source plus a spelled grounding main that instantiates the bounded
  consumer. If G12 exposes a `resolve_user_bound`-over-CtorImage defect,
  that is an implementation-time P0 to fix or to scope explicitly —
  goldens are the discovery mechanism.
- **R-13.4 (overload candidate selection under the lifted fence).**
  `poly_cross_call`'s first-match-wins overload trial (`crosscall.rs:29-40`)
  is unchanged by design: App-declared candidates now participate in the
  trial (the lifted arms can admit them) instead of being skipped by the
  fence's blanket rejection, and ranking still has no ground type to key
  on, so declaration order decides exactly as before the lift. The only
  related pin is the non-App ambiguity canary
  (`tests/phase7_slice3k.rs:326`).

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "crosscall.rs input arm: replace the S1-17.i fence (:346-353) with the guarded declared-App arms (App/App arity guard + head-bind + recurse, App/Generic ctor-image bind per R-13.1 with the None verdict via poly_generic_not_yet_groundable_error, arity guard failure reaches the :354 catch-all), delete the second fence pattern (R-13.2), leave the Var/growth arm untouched; retarget tests.rs:608 (green), :1782 (G5 mismatch), and phase7b_slice12.rs:450 (byte-pin to green Ok); new arm units (mint/len_args faces on a generics-carrying Ctx, head-var conflict unit); create tests/phase7b_slice13.rs with the build_error_bare copy; G2/G5 green",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "crosscall.rs poly_cross_output: add Generic/Array/App output arms rendering through the mapping (rebuilt Generic carries len_args: vec![]; Ref no arm, GenericVariant wildcarded), ast.rs Image doc comment rewrite (comment-only); output-arm units in tests.rs (crosscall.rs has no cfg(test) module); retarget tests.rs:3935 sub-fixture 2 (:3942-3947) in place (flip to green, the poly-body-drop twin of G7); goldens G1/G3/G4/G6/G7/G8/G9 in tests/phase7b_slice13.rs (grounding via existing compose/apply_subst)",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "add required golden G12 (G10.4 source + spelled grounding main instantiating the bounded consumer; fixture bytes pinned at implementation); docs/roadmap: amend the aggregate P7b row in ROADMAP.md and add the lift statement to P7b-higher-kinded-types.md, prefix P7b.S13 everywhere; growth-structure re-check on crosscall.rs; G10 sweep of whatever remains + G11",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
