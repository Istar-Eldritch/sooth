# Phase 7 Slice 3l: a poly body may `call` its own abstract quotation parameter

## Problem (closed)

`poly_call_term`'s `call` arm had two accept cases: a quotation *literal* (`top.quot`,
spliced in place) and a fully ground `Concrete(Type::Quotation(eff))` operand (S3f's R3).
An abstract `PolyType::Quotation` operand, one whose brackets still mention the word's own
type variable (`[ 'T -- 'T ]`), fell to `poly_op_on_variable_error` and rendered
`` `call` is not permitted on a quotation ``, pinned at S3f's exit by its L1.

The spec's opening premise, "the call-site side of this shape already works", was **false**,
and phase 1's recon said so: R9p in `check_poly_call` materialized a literal argument only
against a ground `Concrete(Type::Quotation(eff))` declared slot, so an abstract slot fell to
`reject_quotation_argument` and the headline program was rejected at `main`'s call site with
`` a quotation cannot be passed to `apply`; only `call` accepts one ``. Both boundaries had
to move; the body side landed in phase 1, the argument side in phase 2 (R5).

Headline shape, now building and running:

```
import: intrinsics * ;
: apply ( 'T [ 'T -- 'T ] -- 'T ) call ;
: main ( -- ) 4 [ 1 add ] apply . ;
```

## Requirements (as delivered)

- **R1 Body dispatch, unconditional.** A second arm in `poly_call_term`'s `top.quot`-is-
  `None` branch matches `PolyType::Quotation(ins, outs, ..)` with no `is_inline`/row guard:
  a non-combinator word can never own such a slot (`parse_poly_quotation_inner` rejects a
  row-carrying effect, `check_inline_quotation_requires_inline` rejects `~[ ]` without
  `inline`), so a guard would exclude an unreachable shape. Matches on `&top.pt` and clones
  the two rows; unlike the ground arm's `Type` payload, `Vec<PolyType>` is not `Copy`. Every
  non-quotation operand still falls through to `poly_op_on_variable_error`.
- **R2 Structural rows, variables rigid.** `poly_call_abstract_quotation_param` is the
  abstract twin of `poly_call_ground_quotation_param`: consume `ins.len()` operands
  deepest-first, push `outs`, no body walk and no teardown. Each declared slot is compared
  against the operand's own `PolyType` via derived `Eq` (S3b's L1 discipline). No `Subst` is
  built or consulted mid-body.
- **R3 Located under/mismatch.** Underflow through the existing `underflow_error`
  (`` `call` needs N values, but the stack holds M ``); a structurally unequal operand
  through `poly_rendered_type_mismatch_error`, both sides via `poly_type_str`. The blanket
  message's only remaining source is a genuinely non-quotation operand.
- **R4 Lowering gap, found and closed (phase 1).** `subst_polytype` (`src/ir/driver.rs`)
  carried `PolyType::Quotation(..) => unreachable!`, asserting a quotation-taking word is
  never monomorphized standalone. That premise was already false before this slice for any
  word merely declaring and `drop`ping such a parameter (probe-verified at `HEAD~1`); R1
  added a second, checker-visible path to it. Fixed by mirroring check-side `apply_subst`:
  substitute both rows through `θ`, then ground through `quotation_type`/
  `inline_quotation_type` (these mint a fresh leaked effect, so no lookup-only interning
  like the `Array`/`Ref` arms). A bug fix, not new machinery; `lower_indirect_call` is
  untouched.
- **R5 Call site: ground an abstract declared slot through the completed `Subst`
  (phase 2).** R9p's guard gains a `PolyType::Quotation(..)` arm beside its ground one:
  `apply_subst` grounds the declared row, then the operand materializes through the same
  `materialize_quotation_at_boundary` call and records the same `quot_inputs` entry. The
  input loop splits in two, mirroring `check_poly_combinator_args`: pass 1 unifies every
  non-quotation input (keeping R9p's reject-before-unification), pass 2 grounds and
  materializes each quotation slot against the now-complete `subst`, so the declared
  parameter order does not matter. The split cannot regress the ground path: before this
  slice no quotation input could bind a variable at all. A bare `PolyType::Var` position
  (S3f's L2) keeps rejecting a literal, which is the hazard R9p exists for.

**Out of scope, structurally unreachable rather than guarded.** A row-typed or `~`-declared
abstract quotation parameter on a non-combinator word (chained parser + declaration
rejections above); a poly *combinator* calling its own abstract quotation parameter (checked
through `check_poly_combinator_standalone`, a different splicing path). A declared row
grounding to `Type::InlineQuotation`, and a concrete `Type::InlineQuotation` slot that
`poly_input_is_quotation` admits, stay outside R5's match and keep R9p's rejection rather
than asserting their own impossibility.

**Unruled, left alone.** `poly_unbound_output_error` renders a variable in an *input*
quotation's row as an "output variable"; the renderer is pre-existing and shared.

## Phase 1: checker accept arm, its diagnostics, the lowering gap

Landed in `5baa2b18` (+ `a761d495`). R1–R3 in `src/check/poly.rs`, R4's `subst_polytype` arm
in `src/ir/driver.rs`.

Deviation: the `subst_polytype` fix was not planned as phase 1 work. Leaving it would have
shipped a checker accept arm whose only realistic reachable program panicked at lowering,
so it landed with the arm.

Deviation: exit criterion 1 could not be met here. The R9p gap rejected the headline source
at `main`'s call site, so the phase 1 golden
(`tests/phase7_slice3f.rs::body_boundary_calls_an_abstract_quotation_param`, replacing
`body_boundary_rejects_an_abstract_quotation_param`) forwards the quotation out of a helper
word's return value (`mk_i64`/`mk_bool`) instead, `build_and_run` at two instantiations of
`'T`. It exercises the `subst_polytype` fix as well as the accept arm. The original source
could not be flipped mechanically: it supplied one operand to a two-input word, so lifting
the body rejection would have surfaced an unrelated arity error.

## Phase 2: R9p closure at the argument boundary

Landed in `31e772f3` (+ `b4cc94e8`, `cb4f9aaa`, review fixes).

Deviation: the single-pass version shipped first and was order-dependent, working only when
the quotation slot was declared after the input binding its variable. The two-pass split is
the delivered form, pinned at both the unit and build-and-run level by a reordered fixture.

## Tests (as landed)

Unit, `src/check/poly.rs`:

- `poly_call_on_an_abstract_quotation_param_is_accepted` (flipped from S3f's pinned
  rejection; only the *output* side carries the variable, so a dispatch predicate checking
  the declared inputs were ground would wrongly claim it).
- `..._underflow_is_error`, `..._mismatch_is_error` (R3; the mismatch asserts the full
  rendered diagnostic).
- `..._pops_declared_inputs_deepest_first`, `..._pushes_outputs_in_order` (review round 1
  coverage gap: both accept cases declared one input and one output, so neither could
  discriminate a reversed loop from the correct one). Two heterogeneous slots each,
  mutation-tested: reversing either loop turns its own test red. Push order is checker-only,
  since a quotation effect with two outputs still cannot be lowered (P7.S3m).
- `check_poly_call_materializes_an_abstract_quotation_argument` (flipped from R9p's
  rejection) and `..._declared_first` (reverts the two-pass split to one loop and it goes
  red).
- `poly_call_on_a_variable_local_is_still_error` unchanged: a bare `'T` local is not a
  quotation.

Unit, `src/ir/driver.rs`: `quotation_type_grounds_a_still_abstract_row` pins the arm that
replaced the `unreachable!`.

Golden, `tests/phase7_slice3f.rs`: `body_boundary_calls_an_abstract_quotation_param`
(phase 1, forwarded argument), `headline_apply_accepts_a_literal_quotation_argument` (the
exact exit-criterion source), `headline_apply_accepts_a_literal_quotation_argument_declared_first`
(reordered slots, end-to-end because the reorder changes what lowering receives). Each runs
two instantiations of `'T` (`i64` and `Bool`) so a coincidental single-shape match cannot
pass. `tests/phase7_slice3d.rs::c1_call_on_non_literal_operand_is_accepted` is the renamed,
flipped S3d negative.

No test targets a `~`-declared or row-carrying operand at this arm: there is no witness
program, per the scope note.

## Docs

`docs/roadmap/P7-language-prereqs.md`'s S3l entry is rewritten and marked `[ done ]`,
covering both boundaries. Its earlier phrasing, the body popping and pushing "against the row
grounded through that call's `Subst`", was loose and is corrected: the body compares
structurally with every variable rigid (R2), and a `Subst` is involved only at the call site
(R5). S3f's L1 and S3d's C1-negative bullets are kept as historical record and marked
superseded here.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "checker accept arm for an abstract quotation parameter, its located under/mismatch diagnostics, flipping the pinned unit assertions, and the subst_polytype lowering fix the recon found", "effort": "S", "difficulty": "standard" },
    { "phase": 2, "focus": "close the R9p call-site materialization gap for an abstract quotation position, then the headline build-and-run golden and the green gate", "effort": "S", "difficulty": "standard" }
  ]
}
```
