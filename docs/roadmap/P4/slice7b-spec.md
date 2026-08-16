# Phase 4 Slice 7b: capturing closures

7a gave a quotation a `(code, env)` runtime value but rejected any capturing literal at its four materialization boundaries (struct field, array element, word output, differing-arm join). 7b makes those legal: an `env` that holds the captured references (D4), a checker that proves referents outlive the closure's calls, and a located rejection when they do not. Splicing and force-inlining of quotation-taking words are untouched.

Base `main` @ `c1b1e0a`. Discovery: `phase4-slice7b-brief.md`; Q1/Q2 probe: `phase4-slice7b-q1-probe.md`.

## Problem

7a shipped the representation (`IrType::Quotation = { code, env }`) with `env` hardcoded null and never read, and rejected every capturing literal at a boundary. Two probe findings shaped the design:

1. The checker cannot distinguish a sound escaping capture (`make-b`: over a `&[i64 4]` **parameter**) from an unsound one (`make-a`: over a `[i64 4]` **frame-local**, returned upward, which only panics later in lowering). The one existing test that separates them is `Deriv.owned_root` vs current-frame locals (`check.rs:6108`/`:5519`).
2. 6f's liveness gives opposite answers by evaluation point (admits `make-a` at the boundary, rejects both motivating programs at the call site because erasure sets `quot = None` and drops captured names from `capture_alive_names`), so "just rely on 6f" is unbuildable. A surviving capture set that outlives erasure is genuinely needed.

## Q1 sequencing (floor first, two phases)

- **Phase 1 (floor + escaping single-capture).** Retire the blanket boundary rejection (R12); replace with `owned_root`-vs-current-frame admission (R15). A parameter/global-rooted capture crosses any boundary; a current-frame-local-rooted one is rejected at a word-output (escaping) boundary with a located past-owning-frame error. Env plumbing: a single word-sized reference stored **inline** in the `env` slot (R16/R17). `make-b` runs, `make-a` rejected.
- **Phase 2 (surviving set + same-frame captures).** The two motivating programs capture a same-frame local, never escaping, so the floor's escape guard doesn't reject them, but admitting them safely needs a capture set that survives erasure and feeds `capture_alive_names` past the call site. Adds the multi-capture stack-allocated bundle env, past-last-use rejection, and the escape-guard over the surviving set.

Rationale: Phase 1 is green and sound alone; the inline-single env de-risks the harder bundle/surviving-set work.

## Key anchors

- Blanket rejection retired: `materialize_quotation_at_boundary` guard (`check.rs:6238`) + inline join copy (`check.rs:7204`); `capturing_quotation_error` (`:6204`) deleted.
- Four boundaries (unchanged, D1): word output `check.rs:3675`; `!`/`+!` store `:6912`; quotation param / struct ctor / setter `:7038`; differing-arm join `:7190`–`:7245`.
- Capture-set source (D3, reused): `capture_names`/`capture_names_into` (`:777`/`:783`), cached by `QuotId` via `Provenance::quotation_captures` (`:544`), populated `:7321`. No new capture analysis.
- Admission test: `Deriv.owned_root` vs current-frame locals; precedents at `check.rs:6108` and `check_reference_across_back_edge` (`:5519`, diagnostic `:5501`). A `&T` parameter has `deriv: None` (no `owned_root`); a frame-local borrow carries `owned_root: Some(place)`.
- Liveness: `capture_alive_names` (`:1054`) / `live_derivs` (`:1097`), reach limited by the `Some(QuotRef::Known(id))` guard (`:1066`/`:1077`) — extended in Phase 2.
- `QuotRef` is single-variant `Known(QuotId)` (`:88`), `Slot.quot: Option<QuotRef>` (`:121`); a join of two literals is erased (`quot: None`), so the unioned set rides on a new `Slot` field, not on `QuotRef`.
- Env null: `materialize_quot_value` `Const(env,0)` at `ir.rs:4188`; `lower_indirect_call` loads only `code` at `:4245`; `lower_materialized` `:2370` mints the callee with no env param.
- `contains_reference` (`check.rs:285`) is structurally blind to `Type::Quotation` (`_ => false`), so R22's escape guard reads the surviving set directly, not this predicate.
- `is_copy` (`:239`) treats `Type::Quotation` as Copy; with D4 (references-only env) every admitted closure stays Copy.

## Locked decisions

- **D1.** The four boundaries are unchanged; only the admission rule at them changes. No fifth boundary.
- **D2.** Direct-splice capture needs no new checking (every quotation-parameter position inlines). `call`/`times`/combinators unchanged; splicing is bit-identical, pinned by 7a units (`ir.rs:5214`/`:5228`).
- **D3.** Reuse `quotation_captures`/`capture_names`; add no new capture analysis.
- **D4.** The env holds a **reference** for a captured **aggregate** (`Struct`/`Enum`/`Array`/`OwnedCell` local) or explicit **borrow** (`&x`/`&!x`, `Type::Ref`), never a snapshot — a materialized closure means what the spliced one means because it reads through a live reference (`map` depends on the late read of a captured array).
  - **D4 amendment (scalar snapshot).** A captured **scalar** (not aggregate, not `Type::Ref`, and holds no quotation value in either representation — neither Known `quot.is_some()` nor erased `ty == Type::Quotation`; precisely the class reaching `borrow_of_scalar_local_error`, `check.rs:8210`–`:8214`, after the earlier quotation guard `:8204` and `Ref` guard `:8207` peel off their kinds) is **snapshotted (copied) into the env**, not referenced. Sound: a scalar has no address (can't be borrowed) and R4 forbids rebinding, so snapshot and reference are observationally identical. Consequence: a scalar snapshot occupies the same one-word env slot but never joins the surviving set (R19), R20 liveness, or R23 union.

## Resolved questions (binding)

- **Q1.** Floor first (Phase 1), env-plumbing capability second (Phase 2). Both pieces genuinely needed — no single evaluation point admits the sound programs and rejects the unsound one.
- **Q2a (env representation).** **No heap `^Env` this slice.** 0 captures → null (7a). Exactly one word-sized reference → stored **inline** in the `env` slot. 2+ captures → a synthesized positional **bundle** (layout like `intern_bundle_struct`, `ast.rs:433`, but IR-level `Alloc`/`FieldStore`, no interned `StructDecl`), **stack-allocated** in the materializing frame; a stack bundle dies at return, so 2+-capture is admissible only at an in-frame boundary (R22). References are non-owning → closures stay Copy: no `drop` synthesis, no `^`-site carve-out.
- **Q2b (escaping root).** Parameter/global-rooted capture crosses any boundary; current-frame-local-rooted may not cross a word-output boundary; may cross an in-frame boundary only under Phase 2's surviving-set liveness. Reuses the existing `owned_root` test — no lifetime system (NF2).
- **Q3.** **No `&q`/`&!q` surface syntax, no FnMut** this slice. By-value `call` keeps 7a semantics. Repeated calls of a stored/erased closure are `dup call` (both slots non-owning). D4 forbids closure-local mutable state. Deferred with the heap-env case.
- **Q4 (join union).** A differing-literal join's surviving set is the **union** of both arms', interning a fresh `SurvivingCaptureSetId` (does not mutate either arm's set — keeps the field Copy). **No cap** (bound is already small).
- **Q5.** `capturing_quotation_error` retired; replaced by **past-owning-frame** and **past-last-use** (R24). The four 7a goldens that asserted the old string all capture a bare **scalar**, so under R15 rule 1 / D4 amendment they now **compile** and are re-pointed to positive tests (R25). The real past-owning-frame golden is **T-makea**.

## Requirements (continues 7a's R1–R13; R12 retired)

**Admission (Phase 1)**

- **R14.** Retire both blanket-rejection sites (`check.rs:6238`, `:7204`); delete `capturing_quotation_error`. `body_captures_enclosing` stays as the cheap "captures at all?" gate.
- **R15.** Admission rule: a **four-way classification on capture kind** over `quotation_captures(id)` (D3):
  1. **Scalar value** (D4 snapshot class) — **always admissible at all four boundaries** via env snapshot; no `owned_root` test, no surviving-set entry. Admits the four re-pointed 7a goldens (R25).
  2. **Aggregate value, no deriv** — classify the local's binding by current-frame membership (same test as `:6108`/`:5519`, applied to the captured name). Frame-rooted → rejected at word-output (R24), admitted in-frame per R21. Outer-rooted → admitted anywhere. **Shipped narrowing:** `classify_capture`'s aggregate arm does not distinguish an outer-rooted (parameter/global) by-value aggregate from a frame-constructed one — both reach it with `deriv: None`, and telling them apart needs a provenance tag with the same weight as case 3's `Deriv` tracking. Unbuilt; the arm always returns frame-rooted, so an aggregate-parameter capture is over-rejected at an escaping boundary rather than admitted (sound, just more conservative than this rule as stated).
  3. **Reference capture** (`Type::Ref`) — if it carries a `Deriv`, compare `owned_root` to current-frame locals (current → frame-rooted; parameter/global → outer-rooted). If `deriv: None` (a `&T` parameter/global) → **outer-rooted by construction**, admitted anywhere (matches `check.rs:5527`–`:5531`). Admits **T-makeb** (a `&[i64 4]` parameter); still rejects **make-a**'s frame-local aggregate borrow.
  4. **Captured quotation-typed name** (Known `quot.is_some()`/`ty==Cstr`, or erased `ty==Type::Quotation`) — **rejected as deferred at every boundary**: `error: capturing a quotation value by name is deferred (line {n})` (T-quot-cap-deferred). Golden `:160` is not this case (its inner `[ x + ]` is an inline spliced literal, so only scalar `x` is captured → rule 1).
- **R16.** Env layout per Q2a (0 → null; 1 → inline reference; 2+ → stack bundle). `env` stays `IrType::Ptr` (NF1). Inline single lands Phase 1; bundle Phase 2. Bundle is IR-level, not an interned struct (`src/ast.rs` not sanctioned).
- **R17.** Lowering: `lower_materialized` (`ir.rs:2370`) appends one `IrType::Ptr` env param; captured names resolve to an env read (inline: the env value; bundle: field load at offset). `materialize_quot_value` (`:4170`) builds env from live borrow values instead of `Const(env,0)`. `lower_indirect_call` (`:4245`) passes env as the trailing argument. 0-capture path byte-identical to 7a.
- **R18.** Phase 1 escaping single-capture: a word-output boundary admits exactly one word-sized capture inline (one outer-rooted reference `make-b`, or one scalar snapshot, e.g. T-repoint-join). `make-a` rejected. A 2+-capture escaping closure is rejected: "an escaping closure may capture at most one reference (a heap env is deferred)".

**Surviving set; same-frame (Phase 2)**

- **R19.** Extend the erased `Slot` with an optional `Copy` `SurvivingCaptureSetId` (id into a side table, mirroring `QuotId`/`Provenance::quotation_captures`), **not** an inline `HashSet` (would break `Slot: Copy`, `check.rs:102`). **Shipped shape:** each entry (`SurvivingCapture`) is just `{ name, frame_rooted }` -- the R15 root classification as a bool, no `DerivId`/`owned_root` carried alongside (simpler than originally described here; the interned set itself also carries a `bundle: bool`, R16's env-shape signal, alongside `members`). **Scalar snapshots are never members.** Set at every boundary for aggregate/borrow captures, forwarded by shuffles.
- **R20.** Extend `capture_alive_names`/`live_derivs` to also union an erased slot's surviving set (not only `Some(QuotRef::Known)`-reachable captures). Keeps a frame-local capture's borrow live past the call site so a consume/exclusive-reborrow before the call is rejected past-last-use. Scalar snapshots never enter this.
- **R21.** Admit **frame-rooted** captures at **in-frame** boundaries (struct field, array element, join): the dispatch table and struct-stored-closure programs run, with R20 enforcing referent liveness to each call.
- **R22.** Word-output escape guard: reject a returned quotation (directly or via a returned struct field / array element carrier) whose surviving set includes a **frame-rooted** capture, past-owning-frame. A targeted walk over the surviving set (not `contains_reference`, which is blind to the env). Scalar snapshots absent by construction. **Second, independent arm (review fix):** also reject a returned carrier whenever its surviving set is a **bundle** (`SurvivingSet::bundle`, R16's 2+-total-capture signal) at all, regardless of whether any individual member is frame-rooted -- a 2+-capture closure's stack-allocated env bundle is itself frame-local storage even when every capture it holds is outer-rooted, so returning the carrier still dangles the bundle. Pinned by T-bundle-carrier-outer/-mixed/-scalar.
- **R23.** Join union (Q4): the merged erased slot's surviving set is the union of both arms', **interning a new set** (not mutating in place). No cap. A scalar-captured arm contributes nothing.

**Diagnostics**

- **R24.** Two new located diagnostics replacing `capturing_quotation_error`:
  - **past-owning-frame** — `error: an escaping closure captures {name}, a local of this frame, whose storage does not survive the return (line {n})` (modeled on `reference_across_back_edge_error`). Fires for `make-a` and R22.
  - **past-last-use** — `error: a captured reference to {name} is read after its last use (line {n})`. Fires from R20.
  
  Each names the capture; each has an exact-message test.
- **R25.** Re-point the four 7a goldens (`tests/phase4_quotations.rs:123`/`:143`/`:160`/`:231`), all capturing a bare scalar → now compile under R15 rule 1. Each becomes a **positive** test with a `call` added so it observes the captured value. All land in Phase 1. `:123`/`:160` → struct-constructor boundary (expected `14\n`; `:160` keeps its nested shape); `:143` → reference-store boundary, mirrors the passing `quotation_in_array_element_indirect_calls` oracle with element 1 made capturing (expected `5\n14\n`); `:231` (`pick`) → **T-repoint-join**, a scalar surviving a word-output boundary via inline snapshot (`main` passes `true` → `14`). **T-makea**/**T-makeb** unchanged.

**Regression (Phase 3)**

- **R26.** Every 6a–6f combinator golden and the two 7a splice pins (`ir.rs:5214`/`:5228`, `is_call_instr` `:4855`) stay green and bit-identical; the env param must not turn any spliced literal into `CallIndirect`.

## Non-functional

- **NF1.** `env` stays `IrType::Ptr`, `code` stays `IrType::Code` (opaque; a future WASM lowering realizes them as offset/table index). Bundle offsets are `WORD_WIDTH`-derived, never hardcoded.
- **NF2.** No lifetime apparatus; reuse the existing `owned_root` test.
- **NF3.** No heap `^Env`, no `drop` synthesis, no `^`-site carve-out. Every admitted closure is Copy.
- **NF4.** Splice untouched (D2); R26 pins guarantee it.

## Scope

**In:** widen the admission rule at the four boundaries; build/read the env (inline Phase 1, stack bundle Phase 2); surviving capture set + liveness extension; join union; two new diagnostics; a dogfood.

**Out (deferred):** heap `^Env` / owning-linear closures + their `^`-carve-out and drop plumbing; a 2+-capture **escaping** closure (rejected in Phase 1, R18); `&q`/`&!q` reference-mode `call` + Fn/FnMut/FnOnce split; polymorphic quotation values; opt-in RC (Phase 6); any change to non-capturing behaviour or combinator inlining; the unrelated clause-body rejection (`check.rs:1641`).

## Exit criteria (goldens)

| ID | Test | Kind | Phase | In → out |
|----|------|------|-------|----------|
| T-makeb | `escaping_closure_over_param_ref_compiles_and_runs` | run | 1 | `make-b` over a `&[i64 4]` param, returned+called → spliced value; `CallIndirect` + non-null env |
| T-makea | `escaping_closure_over_frame_local_is_past_owning_frame` | reject | 1 | `make-a` over a frame-local `[i64 4]` → exact past-owning-frame via `assert_eq!` |
| T-env-inline | `materialized_single_capture_builds_inline_env` | unit+IR | 1 | one-capture → env holds the reference, no bundle, one extra `Ptr` param |
| T-multi-esc | `multi_capture_escaping_closure_is_rejected_deferred` | reject | 1 | returned closure over two param refs → exact "at most one reference" message |
| T-quot-cap-deferred | `capturing_quotation_typed_name_is_rejected_deferred` | reject | 1 | quotation bound to a name and captured → exact "capturing a quotation value by name is deferred" (R15 case 4) |
| T-dispatch | `dispatch_table_of_capturing_closures_runs` | run | 2 | array of same-frame capturing closures, indexed+called in-frame → spliced values; `CallIndirect` |
| T-lateread | `struct_stored_closure_observes_later_mutation` | run | 2 | closure in a struct field, captured array mutated, then called → observes mutation (D4); `CallIndirect` |
| T-lastuse | `captured_reference_read_past_last_use_is_error` | reject | 2 | same-frame capture whose referent is consumed before the call → exact past-last-use via `assert_eq!` |
| T-bundle | `materialized_multi_capture_builds_stack_bundle` | unit+IR | 2 | two-capture in-frame → `Alloc` bundle, two `FieldStore`s, env=pointer; word-width offsets |
| T-carrier | `frame_capture_escaping_via_struct_is_past_owning_frame` | reject | 2 | same-frame-capturing closure stored in a returned struct → exact past-owning-frame (R22) |
| T-bundle-carrier-outer | `outer_rooted_bundle_escaping_via_carrier_is_rejected_deferred` (`tests/phase4_quotations.rs:698`) | reject | 2 | a returned carrier's 2-capture stack bundle, every member outer-rooted (no frame-rooted member) → still rejected, exact "at most one reference" message (R22 bundle-escape arm) |
| T-bundle-carrier-mixed | `scalar_and_ref_bundle_escaping_via_carrier_is_rejected_deferred` (`tests/phase4_quotations.rs:724`) | reject | 2 | a returned carrier's bundle of one scalar + one outer-rooted reference (surviving set has only one member) → still rejected on total capture count, not member count |
| T-bundle-carrier-scalar | `scalar_only_bundle_escaping_via_carrier_is_rejected_deferred` (`tests/phase4_quotations.rs:747`) | reject | 2 | a returned carrier's bundle of two scalars (surviving set empty) → still rejected; the empty-member edge case for the bundle marker |
| T-join | `join_of_two_capturing_arms_unions_capture_sets` | run+IR | 2 | two differing capturing literals joined, called while both arms' captures live → correct value |
| T-join-union | `join_capture_union_kills_either_arm_referent_is_past_last_use` | reject | 2 | each arm a distinct referent; killing **either** before the call → past-last-use; both must fire (proves union carries both, R23) |
| T-repoint | three re-pointed 7a goldens: `capturing_scalar_stored_snapshots_into_env` (`tests/phase4_quotations.rs:165`), `capturing_scalar_in_array_element_snapshots` (`:180`), `capturing_scalar_through_nested_quotation_snapshots` (`:200`) | run | 1 | R15 rule 1 admits; each adds a `call` and asserts the value |
| T-repoint-join | `capturing_scalar_at_join_snapshots` (`tests/phase4_quotations.rs:493`) | run | 1 | scalar surviving a word-output boundary via inline snapshot; `true` arm `[ x + ]` x=10, called 4 → 14 |
| T-reg | 7a splice pins stay green | IR | 3 | no spliced literal regresses to `CallIndirect` with the env param (R26) |
| T-dogfood | `capturing_dispatch_matches_spliced_version` | run+IR | 4 | new `examples/*.sth` == hand-spliced twin's stdout; `CallIndirect` + non-null env |
| T-roadmap | ROADMAP 7b marked implemented | doc | 4 | prose exit line |

**Exit note (linear-capture line).** The brief's "dropping a linear-capturing closure disposes its captures" is **vacuously satisfied**: under D4 + Q2a the env holds only references (Copy), so no closure this slice admits is linear. Its substance belongs to the deferred heap-`^Env` work (NF3).

## Mutation-test-required criteria

Prove each guard fails by reverting it in a **throwaway copy** (not a shared worktree).

- **M1 (R15 — T-makea/T-makeb).** Force all-outer-rooted → T-makea stops rejecting (hole reopens); force all-frame-rooted → T-makeb goes red. Proves `owned_root` is the switch.
- **M2 (R20 — T-lastuse).** Drop the erased-slot union from `capture_alive_names` → T-lastuse stops rejecting. *First confirm the R12 exit-row check (`:6108`) doesn't already reject it incidentally, else M2 is a placebo.*
- **M3 (R22 — T-carrier).** Delete the word-output surviving-set walk → T-carrier stops rejecting. *Same R12 placebo check.*
- **M4 (R17 — T-makeb/T-dispatch).** Force the body to ignore its env param → tests go red/panic. Proves env is built and read.
- **M5 (R26 — 7a pins).** Mutate a `Known`-literal `call` to emit `CallIndirect` → pins flip red. Proves the env param didn't reroute splicing.

## Known residual risk (found in post-implementation review, not a scope decision)

`inline_combinator`'s self-tail back-edge still builds its carried-state `outs` as bare `Slot::computed`, dropping any `surviving` set exactly as the getter/array/cell paths did before the review-2 fix. No working adversarial program was found: every self-tail combinator shape the standard library actually uses (e.g. `while`) exits through a conditional join, and `union_surviving` reconstructs the dropped set from the pass-through sibling arm, masking the gap. The one shape that would expose it — a self-tail combinator with no conditional exit at all — hits an unrelated, pre-existing IR-lowering panic (`ir.rs:3119`, `attempt to subtract with overflow` in `Bind`'s `split_off`) that reproduces identically on a closure-free instantiation and predates this slice. Left unfixed: no live exploit, and guess-fixing an unreachable path isn't warranted. Revisit if a future slice adds a self-tail combinator with no conditional exit, or if the unrelated `Bind`/`split_off` panic is ever fixed (which would make this path reachable).

## Sanctioned files

- `src/check.rs` — R14, R15, R19, R20, R21, R22, R23, R24, unit tests.
- `src/ir.rs` — R17 (env param, build, pass), R16 (bundle), unit tests.
- `src/backend/qbe.rs` — only if the one `Ptr` arg needs an ABI adjustment (likely none).
- `src/repl.rs` — only if the new `Slot` field / IR shape requires it.
- `tests/phase4_quotations.rs` — re-pointed goldens (R25) + new goldens.
- `examples/*.sth` — dogfood + hand-spliced twin (T-dogfood).
- `ROADMAP.md` — mark 7b implemented.

No other files.

## Phased delivery

Each phase green under `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.

- **Phase 1 — floor + escaping single-capture.** R14, R15, R16 (inline), R17 (env param + inline build/read), R18, R24 (past-owning-frame), R25. Goldens T-makeb, T-makea, T-env-inline, T-multi-esc, T-quot-cap-deferred, T-repoint, T-repoint-join. Mutation M1, M4 (partial), M5.
- **Phase 2 — surviving set + same-frame.** R16 (bundle), R19, R20, R21, R22, R23, R24 (past-last-use). Goldens T-dispatch, T-lateread, T-lastuse, T-bundle, T-carrier, T-join, T-join-union. Mutation M2, M3, M4.
- **Phase 3 — regression pins.** R26; T-reg.
- **Phase 4 — dogfood + ROADMAP.** T-dogfood, T-roadmap.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Soundness floor + escaping single-capture: retire the blanket capturing-quotation boundary rejection (R14, delete capturing_quotation_error at check.rs:6238 and the inline join copy at :7204); add the four-way admission rule (R15): (1) scalar value capture always admissible via env snapshot (D4 amendment, the borrow_of_scalar_local_error class at check.rs:8210-8214 after the quotation guard :8204 and Ref guard :8207), (2) aggregate value (no deriv) classified by current-frame membership, (3) reference capture by owned_root-vs-current-frame when it carries a Deriv and outer-rooted when deriv is None (matching check.rs:5527-5531), reusing check.rs:6108/:5519, (4) a captured quotation-typed name (Known quot.is_some() or erased ty==Type::Quotation) rejected as deferred (T-quot-cap-deferred); build+read an inline single-reference env (R16/R17) via one extra Ptr param on the materialized IrFunc (lower_materialized ir.rs:2370), building env in materialize_quot_value (ir.rs:4170, replacing Const(env,0) at :4188) and passing it in lower_indirect_call (ir.rs:4245); reject 2+-capture escaping closures as deferred (R18); add past-owning-frame (R24, modeled on reference_across_back_edge_error :5501); re-point the four 7a scalar goldens (:123/:143/:160/:231) to positive tests (R25). Goldens T-makeb, T-makea, T-env-inline, T-multi-esc, T-quot-cap-deferred, T-repoint, T-repoint-join; mutation M1, M5.",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Surviving capture set + same-frame in-frame captures: add an optional Copy SurvivingCaptureSetId to Slot (R19, side table, not an inline HashSet which breaks Slot: Copy at check.rs:102; mirrors QuotRef::Known and Provenance::quotation_captures :437/:544), set at every boundary for aggregate/borrow captures only and forwarded by shuffles; extend capture_alive_names (:1054) and live_derivs (:1097) to union an erased slot's surviving set (R20); admit frame-rooted aggregate/borrow captures at in-frame boundaries (R21); add the stack-allocated multi-capture bundle env (R16, layout like intern_bundle_struct ast.rs:433, IR-level); add the word-output escape guard walking the surviving set (R22, since contains_reference :285 is env-blind); union both arms' sets at a differing-literal join by interning a new set (R23, :7195); add past-last-use (R24). T-join-union captures a distinct referent per arm and rejects when either is killed. Goldens T-dispatch, T-lateread, T-lastuse, T-bundle, T-carrier, T-join, T-join-union; mutation M2, M3, M4.",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Regression pins: confirm every 6a-6f combinator golden and the two 7a splice-vs-indirect pins (call_of_literal_emits_no_call_instr ir.rs:5214, times_lowers_... :5228, is_call_instr :4855) stay green and bit-identical now that the materialized body carries an env parameter (R26, NF4). Golden T-reg.",
      "difficulty": "standard"
    },
    {
      "phase": 4,
      "focus": "Dogfood and ROADMAP: add a capturing-closure example under examples/ with a hand-spliced twin as the parity oracle, and a parity golden asserting identical stdout plus a CallIndirect with a non-null env (T-dogfood); mark Phase 4 Slice 7b implemented in ROADMAP.md (T-roadmap).",
      "difficulty": "standard"
    }
  ]
}
```
