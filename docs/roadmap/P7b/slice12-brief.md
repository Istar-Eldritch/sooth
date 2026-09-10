# P7b.S12 brief — poly-body App-dispatch output rendering (SOO-39)

- Date: 260911. Base: `d7559bb` (main tip; the `soo-39` worktree was
  fast-forwarded to it — the branch had no unique commits — so every citation
  below is at the post-S13 poly.rs partition and the post-S6d-PREREQ checker).
  Suite green at base: 3463 passed / 0 failed.
- Provenance: Linear [SOO-39](https://linear.app/ordfruma/issue/SOO-39)
  ("Poly body: App-dispatch call loses its outputs (blocks bound-generic
  slice consumers)"), priority 2, project P7b. The defect was first recorded
  by probe round S6d-4 (260907) and deferred by S6d-PREREQ as its
  out-of-scope item (3) — "its own slice". This is that slice: P7b's next
  number (S12; S13 is taken by the SOO-32 poly.rs partition, filed under
  P7-language-prereqs).
- Sources: probe round [slice12-probes](./slice12-probes.md) (verbatim
  rounds A–G plus probe H — 9 fixtures), byte-exact stderr baseline
  `probes/s12_baseline.md`, fixtures
  `probes/s12_*.sth`, invocation `cargo run -q -- build probes/<name>.sth`
  (probe H adds `--manifest tests/fixtures/sooth.pkg`). Independent
  verification prober verdicts: see the recon section at the end.
- Companion roadmap entries: `P7b-higher-kinded-types.md` §P7b.S6d /
  §P7b.S6d-PREREQ (the deferral), `slice6d-prereq-spec.md` "Deferred /
  out of scope" item (3).

## Problem

A bound-generic body (`['It: Cursor]`) that dispatches a trait member whose
row is App-headed (`next ( 'It['T] -- Option['T] 'It['T] )`) fails at check
with a stack-effect mismatch — or, worse, checks with a silently mis-typed
value. Evidence (slice12-probes):

1. **The S6d-4 symptom, byte-exact (A2).** A drain fold over `'It[i64]`
   fails with `` `add` needs 2 values, but the stack holds 0 `` — the exact
   message shape S6d-4 recorded. The drain's remaining values are on the
   stack; the operator path just cannot see them (defect 3 below).
2. **The outputs are not lost — they are mis-typed (B/G).** The dispatch
   leaves all values, but the concrete-ctor output (`Option['T]`) carries a
   **leaked member-space variable**: `Option['?1]` (out of range in a
   one-variable caller). The App-headed remainder (`'It[i64]`) renders
   correctly.
3. **The operator symptom mechanism.** `poly_delegate_op` extracts the
   *maximal concrete suffix* of the operand run and runs the concrete
   check on it. A leaked-var payload is non-concrete, so the suffix is
   short and the operator reports underflow ("holds 1"/"holds 0"). This is
   what the S6d-4 note personified as "loses its outputs".
4. **Silent mis-typing when the ids land in range (C).** With the element
   variable written first (`['E 'It: Cursor]`), the leaked member id 1 is
   the caller's `'It` — a `* -> *` constructor variable — so the `Some`
   payload types as `'It`. Caught in the probe only because the arms
   disagreed; a body shaped to agree would carry the wrong type onward.
   **This face is a correctness gap, not a diagnostics gap.**
5. **The mask (F).** The leak is invisible exactly when the caller's effect
   mentions its element variable second: the leaked member id (1) then
   reads as the right caller variable. `lib/core/iterator.sth`'s
   `fold`/`for_each` stand on this coincidence ('It=0, 'T=1 by effect
   mention order; the member row's element is also id 1). Any consumer with
   a concrete element, an extra leading variable, or a different effect
   order breaks. The shipped Iterator consumers are the only reason this
   looked like a narrow S6d-gate instead of a general bound-dispatch
   defect.

Independent of the leak, the same roadmap sentence names a second facet —
poly bodies **cross-calling** poly words over compound receivers — which is
a *located* fence today (E: "a higher-kinded application in a
cross-called polymorphic word is not yet supported", S1-17.i), not a loss.
It does not gate the drain-fold/`fold` shapes (member dispatch, self-call,
and quotation call only) and is not this slice's fix; see Ruling 4.

## Adjudicated mechanism (probe round; code citations at `d7559bb`)

- **Root cause: `render_member_decl` (`src/check/poly/ground.rs:197`) has no
  `PolyType::Generic` arm.** `poly_trait_member_call` (same file, :1516) —
  the bound-generic body's member-dispatch route — unifies the member row's
  App-headed input against the caller's operand (correctly binding the
  member header var and element vars in `bindings`), truncates the
  operands, and re-pushes the declared outputs through `render_member_decl`.
  That function renders `PolyType::App` (abstract head) head-and-args
  through the bindings, but `PolyType::Generic` — how a concrete ctor
  applied to variables (`Option['T]`, `Step['T 'It['T]]`) is spelled in
  `PolyType` space — falls into `other => other.clone()`: **args stay in
  member-variable space**. The member row's element var (member id 1; the
  trait header var is id 0, per `TraitDecl`'s single-var contract and
  `dispatchable_input_pos`) lands verbatim in the caller's id space.
- **Id ordering fact (probe F/G):** a poly word's `ty_var_names` are
  interned in the **effect's mention order**, not the bound list's written
  order. `['It: Cursor 'T] ( 'T 'It['T] -- … )` has 'T=0, 'It=1. The leaked
  member id 1 therefore reads as `'It` in that word — in-range but wrong.
- **The other dispatch routes are correct.** The splice route
  (`resolve_splice_member_call`) grounds member sigs through
  `ground_member_sig_via_theta` → `apply_subst`, whose `PolyType::Generic`
  arm substitutes args recursively and mints monomorphs; the mono route
  (`resolve_mono_member_call`) grounds a concrete-impl member through
  `ast::ground_member_type` with a concrete-effect fallback (ground.rs
  :1244-1256) and routes a generic-impl winner through `check_poly_call` —
  none of them touch `render_member_decl`. Probe H: an
  `impl: Cursor for List` member (`Option` row over `List['T]`) **checks
  clean today** — only the bound-generic consumer fails. The defect is
  confined to the symbolic body-check render (the single production push at
  ground.rs:1678).
- **The diagnostics twin has the same gap.** `substitute_member_var`
  (ground.rs:254) — the member-space→dispatch-var rewrite used only in
  error-text rendering (its two callers, ground.rs:433 and :1600, build
  candidate-shape strings) — handles `App` but not `Generic`, so member
  sigs with concrete-ctor applications render wrong in those messages.
- **Self-recursion needs no fix (D).** The self-call arm is a structural
  pointwise match against the walk's own sig; App-vs-App operands compare
  correctly today.
- **The cross-call fence is separate (E).** `poly_cross_match`'s S1-17.i arm
  rejects any App slot in a cross-call; `poly_cross_output`'s `_ =>` arm
  rejects any compound output. Both are located rejections with their own
  tests (`src/check/poly/tests.rs` pins the fence text). Lifting them is
  real design work (symbolic App matching, compound output rendering
  through the mapping, interaction with R6's growth ban) that no S6d
  consumer shape needs.
- **Non-blocking observations.** (i) Underflow/type-mismatch errors inside
  a poly body print `note: declared ( -- )` — `ctx.effect()` is the poly
  word's empty placeholder (`WordDef.effect` doc: "For a polymorphic word
  this is left empty"). Real, but many existing goldens pin that placeholder
  byte-exactly in poly contexts (`tests/phase7b_slice8b.rs` ×5+,
  `tests/phase0.rs`, `tests/phase7_slice3c.rs`), so fixing it is a
  deliberate multi-golden text change — deferred (Ruling 5). (ii) The poly
  checker has no provenance channel over `PolySlot` (no `Deriv`), so a
  fixed renderer still hands a slice remainder to a poly body with no
  borrow propagation — an S6d soundness question, not S12's (recorded for
  S6d in the spec's deferred section).

## Rulings (proposed; confirm at spec review round 1)

- **R1 (direction).** Fix the render, not the diagnostics. Face C is a
  correctness gap: a program can check with a constructor-variable-typed
  value. Sharpened errors alone would leave the mis-typing either silent or
  merely rejected.
- **R2 (scope).** `render_member_decl` gains a `PolyType::Generic` arm —
  render `args` recursively through the same `render` closure, keep
  `is_enum`/`idx`/`module`/`name` verbatim, carry `len_args` verbatim
  (S2-5: a member row declares no length variables, so a `Len::Var` there
  is unrepresentable and unreachable). `substitute_member_var` gains the
  mirror arm for diagnostics. No other dispatch route changes; the four
  other `push`/render sites named in S6d-PREREQ's REQ-4d site 4 are
  concrete-route pushes already correct.
- **R3 (fallback).** An unbound member variable inside Generic args renders
  exactly as the `Var` arm's fallback does: `PolyType::Var(var)` (the
  caller's bound variable). This is the existing convention
  (`render_member_decl`'s own doc) and the only defined answer for a
  member-local the operands never pinned.
  *(Amended by the review round, 260911 — see the spec's R3: this is now
  the two-tier rule. `unify_member_operand`'s structural-equality tail
  (`ground.rs:187`) admits ctor-headed inputs with **no binding**, but
  those programs compile today — the per-site mint binds the pinned var
  via `unify_poly_input`'s Generic arm (`unify.rs:343-374`) — so the
  verbatim tier keeps them byte-stable; the unscoped fallback would
  re-type the pinned payload as the header variable and break them.
  `Var(var)` remains the answer only for output-only locals, which never
  compile on either route.)*
- **R4 (fences — what does NOT move).** The cross-call App fence
  (S1-17.i) and `poly_cross_output`'s compound-output rejection stay
  untouched, byte-identical tests included; their lift is recorded as a
  follow-up slice (**[SOO-60](https://linear.app/ordfruma/issue/SOO-60)**; it is the remaining facet of the
  roadmap sentence, and
  S6d does not need it). No S6d work lands here: no sentinel-substitution
  grounding, no slice impl, no `PolySlot` provenance. The S6d-6 concrete
  fence, the S2-6 fence, and all S8/S8b/S8c member machinery are untouched.
- **R5 (diagnostics fence).** The `note: declared ( -- )` placeholder in
  poly-body underflow notes is NOT fixed in this slice (multi-golden text
  churn; recorded as its own tiny slice candidate —
  **[SOO-61](https://linear.app/ordfruma/issue/SOO-61)**). No diagnostic text
  changes except where the fix makes previously-garbled member-sig
  renderings correct (G7) — and those messages are today unreachable-with-
  correct-content, so no existing golden pins their Generic shape.
- **R6 (behavioral fence — the coincidence contract).** Programs that check
  today **by id coincidence** (`fold`/`for_each` over List and Range, and
  any other consumer whose element variable sits at id 1) must produce
  **byte-identical rendered types** post-fix: for those shapes the Generic
  arm's rendering of member Var(1) through bindings equals what the verbatim
  clone produced, because the binding map already held `(1, Var(1))` — the
  arm is a no-op there. The full existing suite is the regression gate;
  the `for_each`/`fold` List and Range goldens (`tests/phase7b_slice8.rs`)
  are the named canaries.
- **R7 (no new capability).** The fix is a rendering correction inside the
  existing unification/bindings machinery. No new `PolyType` variants, no
  new unification rules, no changes to `unify_member_operand` or the
  obligation record. If a member row shape is found that the arm cannot
  render, it is a located error, never a silent clone (the only silent
  shape today is the bug itself).
  *(Amended by the review round, 260911 — see the spec's R7 for the
  tightened wording: the render changes observable behaviour only where
  today's render was already the leak; structurally-pinned compiling
  programs render byte-identically, output-only rows never compile on
  either route, and the unscoped fallback would have broken today-green
  bodies — the face-2 class this slice kills.)*

## Goldens (proposed; `tests/phase7b_slice12.rs`; paper tests in [slice12-paper-tests](./slice12-paper-tests.md))

| Golden | Fixture | Today | Post-fix |
| --- | --- | --- | --- |
| G1 | s12_a (drain fold, Option row) | underflow at `add` ("holds 1") | build_ok (checks clean; no `main`/impl needed) |
| G2 | s12_a2 (add on the payload) | underflow ("holds 0") — S6d-4's exact message | build_ok |
| G3 | s12_b (residual exposure) | `Option['?1]` in the mismatch | build_ok (residual `i64 'It[i64] Option[i64]` matches declared) |
| G4 | s12_c (in-range mis-typing) | arms disagree (`'E` vs `'It`) | build_ok (payload is `'E`; arms agree) |
| G5 | s12_f (id-space control) | `Option['It]` vs `Option['T]` | build_ok (Generic arm renders through bindings) |
| G6 | s12_h (end-to-end: `impl: Cursor for List` + bound-generic drain, prints 6) | fails in `drain` (impl checks clean) | build+run, stdout `6` — the slice's exit criterion |
| G7 | diagnostics twin: a member-dispatch error message that renders a member sig containing `Option['T]` | garbled member-space rendering | the member type renders in caller space, byte-pinned |
| G8 | s12_e (cross-call App fence) | located rejection, byte-exact | byte-identical (R4's untouched fence) |
| G9 | `for_each`/`fold` over List + Range (`tests/phase7b_slice8.rs`) | green | green (R6 canary; no symbol-diff harness wired) |
| G10 | existing suite | green (3463) | green (R6's global gate) |

Units beside each changed site (`render_member_decl`, `substitute_member_var`
— both in `src/check/poly/ground.rs`, tests beside the band per the S13
layout): Generic args render through bindings; unbound member var falls back
to `PolyType::Var(var)`; `len_args` carried verbatim; nested
`Generic`-in-`Generic` and `App`-inside-`Generic`-args recurse; `QuotLit`
stays unreachable; `substitute_member_var`'s twin arm mirrors all of it.
Naming `thing_condition_expected`.

## Open questions

- **Cross-call App lift timing.** R4 defers the S1-17.i fence lift. If
  S6d's consumer work turns out to need poly-word-to-poly-word calls over
  compound receivers (e.g. a user helper word), the lift becomes S6d-gating
  and should take its own probe round first — the interaction with R6's
  growth ban (a bare-var callee input facing an App-typed operand) is the
  hard part, not the App-vs-App matching.
- **`GenericVariant` in a member output.** Unconstructible outside
  eliminator arms (R3.5); the spec should pin the arm's posture (unreachable
  assertion vs verbatim clone) — the codebase's R3.5 convention says
  `unreachable!`, and no fixture can reach it.
- **The `note: declared ( -- )` placeholder** (R5): candidate tiny slice;
  needs a golden-churn inventory first (phase7b_slice8b's five pins are the
  start).
- **Poly-body borrow propagation for slice remainders** (`PolySlot` has no
  `Deriv`): recorded for S6d, not S12. If S6d's slice consumers hit the
  exclusivity story, that is where it lands.

## Recon addendum — independent verification (260911)

An independent `prober` subagent re-verified the load-bearing claims on a
scratch `git archive HEAD` extract at `d7559bb` (worktree untouched). Full
verdicts:

- **C1/C2 (root cause + diagnostics twin) CONFIRMED.**
  `render_member_decl` (ground.rs:197-232) has no `PolyType::Generic` arm;
  its only production caller is `poly_trait_member_call`'s output push
  (ground.rs:1678). `substitute_member_var` (ground.rs:254-277) same gap;
  both its callers (ground.rs:433, :1600) build error-text only.
- **C3 (probe battery) CONFIRMED 8/8 byte-identical**, stderr + exit codes
  against `probes/s12_baseline.md`. One process finding: a superseded draft
  of probe F (`s12_f_id_coincidence_control.sth`, bound-list-order
  assumption in its comment) was still on disk without a baseline entry;
  deleted — the canonical fixture is `s12_f_id_coincidence_space.sth`, whose
  baseline entry it byte-matches.
- **C4 (id ordering) CONFIRMED in full.** `intern_ty_var`
  (parser.rs:2043-2049) ids by first mention; the bound bracket is a side
  table attached after `parse_poly_effect`, "so ids stay effect-derived"
  (parser.rs:3344-3346, :3497-3500). Member side: the trait header var is
  pre-interned at 0 (parser.rs:4024-4046, with a `debug_assert_eq!(id, 0)`),
  so the row's element is member id 1. The prober added two Step-row
  empirics: a fold-layout caller ('It=0, 'T=1) renders `Step['T 'It['T]]`
  (today's leak reads correct — the R6 coincidence, proven at the Step
  row); a swapped caller ('T=0, 'It=1) leaks verbatim as `Step['It
  'T['It]]` — nested App included, never re-rendered. So the shipped
  Iterator consumers' Step scrutinees carry leaked member ids today.
- **C5 (confinement) conclusion CONFIRMED; one mechanism detail corrected**
  (folded into the mechanism section above): `resolve_mono_member_call`
  grounds via `ast::ground_member_type` + concrete-effect fallback and
  routes generic-impl winners through `check_poly_call`, not
  `apply_subst` directly. `apply_subst` itself substitutes Generic args
  (unify.rs:711-727). The App fence is crosscall.rs:346-352 (`S1-17.i`
  comment); `poly_cross_output`'s compound rejection is :387-396.
- **C6 (post-fix hand-trace) CONFIRMED.** For probe F the bindings are
  pre-bind `(0, Var(1))` plus operand binds `(0, Var(1))`, `(1, Var(0))`;
  the proposed Generic arm renders Option's arg `Var(1)` → `Var(0)` →
  caller `'T` → `Option['T]`; residual then matches declared → probe F
  goes green. No source edited.
- **C7 (base health) CONFIRMED.** Scratch-extract `cargo test`: 3463
  passed, 0 failed.
