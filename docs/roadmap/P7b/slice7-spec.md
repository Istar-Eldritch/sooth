# P7b.S7 — quotation effects over type constructors (the `call` extension), condensed

> Implemented reference. The delivery plan (numbered requirements, risk tables,
> phased steps, phases JSON) is retired; the code and the **Implementation**
> section below supersede it. The frozen companion docs stay as-is:
> [slice7-brief](./slice7-brief.md) (recon, adjudicated mechanism, scope) and
> [slice7-paper-tests](./slice7-paper-tests.md) / [slice7-probes](./slice7-probes.md)
> (fixture text and verbatim probe log). Base `d7ba59c`.

## Why

S2 shipped `Functor.map` (an HKT output at a quotation's top level, grounded via
`apply_subst`'s `App` arm) but fenced richer container traits at parse time. Two
fences blocked the next rung:

1. **`Monad.bind`** — `bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] )` puts an App
   *inside* a quotation row. `member_shape_is_supported`'s `Quotation` arm rejected
   any row where `member_quotation_row_mentions_app` fired, before `check`, before
   dispatch — the trait never became a `TraitDecl`.
2. **`Applicative.ap`** — `ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] )` puts a
   quotation *as the argument* of a variable-headed App. `parse_poly_app_arg`
   rejected this unconditionally, independent of traits. A named constructor's
   application list (`Box[[i64 -- i64]]`) has no such fence — the restriction was
   specific to a type-*variable* head.

The recon round's original premise — that `poly_call_abstract_quotation_param`
needs new logic reusing `PolyType::App`'s structural equality — was **wrong**.
`poly_call_abstract_quotation_param` never sees an unresolved App at all: an
`impl:` member's body is grounded against its target *before* body-check runs, and
`ground_member_poly`'s existing `App` arm already dissolves a row-nested
`'F['U]` into a plain `Generic` at parse time. What genuinely needed doing was
lifting the two parser fences and verifying (not extending) the already-shipped
grounding — `ground_member_poly` (callee side) and
`unify_member_operand`/`render_member_decl` (caller side, already exercised by
`Functor.map`) — against this one previously-unparseable shape.

## What shipped

**Fence #1 lifted (`bind` admitted).** `member_shape_is_supported`'s `Quotation`
arm no longer rejects a row containing an App headed by the trait's own type
variable (`head == 0`); a row-nested App headed by anything else still fences,
message corrected from "App-free" to name the narrow rule. `ground_member_poly`'s
`Quotation`→`App` recursion (pre-existing) dissolves `bind`'s row-nested `'F['U]`
into `Generic{Option, [Var(U)]}` at parse time — no new grounding code.
`unify_member_operand`/`render_member_decl` (pre-existing, already exercised by
`Functor.map` through a shared bound) correctly unify/render the same shape for a
generic caller dispatching `bind` through a `'T: Monad` bound — verified by three
new units, zero production changes.

**Dispatch + IR.** `bind` (fixture-local `trait:`, never shipped in `lib/`) is
`inline`, dispatches per constructor (`Option`/`Result`), and splices with zero
call frame — equivalent to a hand-written inline call. Each impl
short-circuits on the empty/error constructor without consuming the quotation and
applies it exactly once on the full/ok constructor, structurally enforced by the
type checker.

**`Applicative.ap` — measured, not grounded; deferred.** Lifting fence #2 is
sufficient to *parse* `ap`'s declaration, but declaring `impl: Applicative for
Option` (no call site needed) hits a second, independent, checker-level fence:
`audit_poly_input_quotation`'s `Generic` arm (`src/check/audits.rs:431`) calls into
`reject_poly_quotation_anywhere`'s own `Quotation` arm (`src/check/audits.rs:484`),
which rejects a quotation nested inside a type application's argument list — the same
rejection class as the pre-existing `Box[['T -- 'T]]` precedent. This is a
different shape from `bind`'s: `bind`'s quotation is a *direct* member parameter
(the App only inside the row); `ap`'s quotation is nested *inside* `'F`'s own
application. Grounding `ap` needs new logic in that audit — outside this slice's
scope. The fence #2 lift was **reverted** (`parse_poly_app_arg` is byte-identical
to pre-Phase-4 HEAD); `ap` ships no golden, recorded as future work gated on
whichever slice takes up `reject_poly_quotation_anywhere`'s
constructor-of-quotation case.

**REQ-11 measurement (recorded, not fixed here).** `unify_member_operand`'s
fallback arm has no case pairing a declared `Quotation` against a caller's literal
`PolyType::QuotLit` operand. Confirmed real via a minimal poly-body fixture; a
permissive fix built clean in isolation but regressed a live S6 invariant
(`tests/phase7b_slice6.rs`'s `poly_body_quotation_literal_member_operand_is_located_error`)
by letting the case through to a worse, unlocated downstream error. Reverted;
recorded as a unit (`unify_member_operand_rejects_a_literal_quotation_operand`,
`src/check/poly.rs`) — deferred, belongs to S6's own scope. Orthogonal to this
slice's goldens: `bind`'s dogfood call is a mono call with explicit instantiation
(`bind[i64 i64]`), which never produces a `QuotLit` operand. The gap is inherited
from S6 (shared with `map`, pinned by `tests/phase7b_slice6.rs`'s
`poly_body_quotation_literal_member_operand_is_located_error`), not introduced
here, but it does block the literal-quotation spelling of `bind`/`and_then` inside
a generic body (e.g. `chain['F: Monad] ( 'F[i64] -- 'F[i64] ) [ half ] bind ;`
fails to type-check today) — not merely "orthogonal to the goldens".

## Load-bearing rulings

- **No new grounding code.** `poly_call_abstract_quotation_param` never sees an
  unresolved App; `ground_member_poly` dissolves it into a `Generic` before
  body-check runs; a non-`Generic` impl target is already a located error, so the
  "`'F` still abstract" case is structurally impossible. Two already-shipped
  mechanisms were verified against a previously-unreachable shape, not extended.
- **`ap` is deferred by measurement, not by decision.** It fails a different,
  independent audit (`reject_poly_quotation_anywhere`), not the mechanism this
  slice touches.
- **Invariant-bearing guards audited, not broken.** `ground_member_type`'s `App`
  arm stays correctly unreachable (`fence_member_app_against_concrete_target`
  fences an App anywhere in a *concrete*-target signature, rows included, before
  it). `apply_subst`'s `head`-indexing is safe by construction (an App's head id
  is always allocated within the same `PolySig`'s own variable space). Both got
  doc-comment corrections only, no logic change.
- **Diagnostics are behaviour.** `app_in_member_quotation_row_error`'s message
  corrected for the one shape that still fires post-lift (a member-local-headed
  App in a row); the corrected text is pinned by G4b, not left to drift.

## Goldens

All in `tests/phase7b_slice7.rs`. Fixture text in
[slice7-paper-tests](./slice7-paper-tests.md).

| Golden | Test name | Behaviour |
| --- | --- | --- |
| G1 | `monad_bind_declares_app_in_quotation_row` | the row-nested-App `Monad` trait declaration builds and runs; exit 0 |
| G2 | `option_bind_dispatches_and_short_circuits` | dispatches per constructor; short-circuits on `None`; distinguishable output on every arm; no `bind` symbol emitted (zero-frame splice) |
| G3 | `result_bind_dispatches_and_short_circuits_on_err` | dispatches on `Result`, distinct `impl:` from Option's (constructor-keyed); short-circuits on `Err` |
| G4a | `kind_incorrect_app_in_quotation_row_is_error` | regression pin: pre-existing `arrow_var_used_bare_error` fires independently of the row fence; located, exit 1 |
| G4b | `member_local_headed_app_in_quotation_row_is_still_unsupported` | the real narrowness witness: a member-local-headed (not trait-var-headed) App in a row is still rejected post-lift; located, exit 1, corrected message |
| Reg | `named_ctor_of_quotation_argument_unchanged` | `Box[[i64 -- i64]]` path byte-identical (probe P5) |

`Applicative.ap` ships no golden (deferred; see above).

## Key units

- `member_quotation_row_admits_app_expected` (`src/parser.rs`) — fence #1 lift.
- `kind_incorrect_app_in_quotation_row_is_located_error` (`src/parser.rs`).
- `ground_member_poly_quotation_row_app_dissolves_to_target_generic` (`src/ast.rs`)
  — callee-path verification.
- `unify_member_operand_app_row_slot_binds_dispatched_ctor`,
  `render_member_decl_app_row_slot_renders_into_caller_space`
  (`src/check/poly.rs`) — caller-path verification.
- `unify_member_operand_rejects_a_literal_quotation_operand`
  (`src/check/poly.rs`) — REQ-11 measurement, deferred to S6's scope.

## Growth-structure re-check

Re-run at Phase 5 exit on every file touched: `src/parser.rs` (15,326 lines; this
slice's own contribution is one conjunct deletion, five message/comment
corrections, and test-only additions — no new responsibility added; of
CLAUDE.md's five signals, import-divergence and forced-circularity are silent,
which is why the file stays whole despite its size, but a full five-signal audit
of the file as a whole is not re-run here — only the slice's own delta),
`src/ast.rs` / `src/check/poly.rs` (comment/test-only diffs), `src/check/audits.rs`
(zero diff — cited and measured for the `ap` deferral, never edited),
`tests/phase7b_slice7.rs` (new file, single golden-test responsibility), one
test retired/inverted in `tests/phase7b_slice2.rs` and its parser-unit twin
in `src/parser.rs`, roadmap docs (prose-only). No split needed anywhere.

## Implementation

| Area | Commit | Key files |
| --- | --- | --- |
| Phase 1 — lift fence #1, retire S2-15.d tests, audit row invariants, kind-correctness (REQ-1, REQ-3, REQ-4) | `f0119f4` | `src/parser.rs`, `src/ast.rs` (doc), `src/check/poly.rs` (doc), `tests/phase7b_slice2.rs`, `tests/phase7b_slice7.rs`, `docs/roadmap/P7b/slice2-spec.md` |
| Phase 2 — verify the already-shipped grounding against the row-nested-App shape, no new code (REQ-5) | `31b0d29` | `src/ast.rs`, `src/check/poly.rs` (units only) |
| Phase 3 — dispatch + IR: `bind` over Option/Result (REQ-6, REQ-7, REQ-11) | `a3d5b9d` | `src/check/poly.rs`, `tests/phase7b_slice7.rs` |
| Phase 4 — `Applicative.ap` measured, not grounded, deferred (REQ-2, REQ-8, OQ-2) | `0065eef` | `src/parser.rs` (fence #2 lift then revert, net byte-identical), `docs/roadmap/P7b/slice7-spec.md` |
| Phase 5 — roadmap correction, growth re-check, final gate (REQ-9, REQ-10) | `a4bef17` | `docs/roadmap/P7b-higher-kinded-types.md`, `docs/roadmap/P7b/slice7-brief.md` |

## Residual / future work

`Applicative.ap` is gated on whichever slice takes up
`reject_poly_quotation_anywhere`'s constructor-of-quotation case
(`src/check/audits.rs:431`) — a quotation nested inside a type application's own
argument list, distinct from a quotation as a direct member parameter. Not an S7
regression; recorded here as a forward gate, per the roadmap's S7 entry.
