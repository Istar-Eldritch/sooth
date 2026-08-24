# Phase 7 Slice 3l: a poly body may `call` its own abstract quotation parameter

## Goal

A plain (non-`inline`, non-combinator) polymorphic word may now `call` a quotation-typed
parameter it declared, from inside its own body, when that parameter's brackets still
mention the word's own type variables (`[ 'T -- 'T ]`). The call-site side of this shape
already works (`unify_poly_input`'s `Quotation` arm binds the row pointwise); only the body
side was closed. The headline shape builds and runs:

```
: apply ( 'T [ 'T -- 'T ] -- 'T ) call ;
: main ( -- ) 4 [ 1 add ] apply print ;
```

Before this slice, `poly_call_term`'s `call` arm had two live accept cases: a quotation
*literal* (`top.quot`, spliced in place) and a fully **ground**
`Concrete(Type::Quotation(eff))` operand (S3f's R3, `poly_call_ground_quotation_param`).
Every other operand, including a `PolyType::Quotation(..)` still carrying a variable, fell
through to `poly_op_on_variable_error`, rendering `` `call` is not permitted on a quotation``.
That rejection was deliberately pinned at S3f's exit as out of scope (L1) and is asserted as
the *expected* behaviour by two tests, both of which flip to accept cases in this slice.

## Scope

**In scope.** A non-`inline`, non-combinator polymorphic word's `call` on its own declared
quotation parameter whose declared effect (`PolyType::Quotation(ins, outs, false, None,
None)`) still mentions a variable, once the caller's instantiation has bound every variable
the row mentions. This is the direct mirror of S3f's R3 for the ground case, one level
earlier in the pipeline: pop the quotation, consume its declared inputs deepest-first, push
its declared outputs, with no body walk and no teardown (there is no body behind the value).

**Out of scope (pinned, not silent — but structurally unreachable, not guarded).**

- A **row-typed** or `~`-declared (`is_inline`) abstract quotation parameter never reaches
  this arm at all, on any word: `is_combinator` (`src/check/combinators.rs:155-166`) rejects
  a `~[ ]` parameter at declaration unless the word also declares `inline`, and
  `parse_poly_quotation_inner` (`src/parser.rs:3049`) rejects a row-carrying effect
  (`row_in`/`row_out` both `Some`) unless `is_inline` is set. Chained, a non-combinator word
  can never own a `PolyType::Quotation` with `is_inline = true` or either row `Some` in the
  first place, so R1 does not need to guard against it — there is no live shape to exclude,
  only Slice 10a's combinator machinery, which is out of scope for a different reason (next
  bullet). No new diagnostic wording is introduced for this non-shape.
- A poly **combinator** calling its own abstract quotation parameter: combinators are
  checked through `check_poly_combinator_standalone`, a different term-splicing path, not
  `poly_call_term`'s ordinary body walk this slice touches.
- No new diagnostics infrastructure beyond arms that mirror
  `poly_call_ground_quotation_param`'s existing `underflow_error` /
  `poly_rendered_type_mismatch_error` pattern.

## Rulings

**R1: dispatch on the abstract quotation operand.** In `poly_call_term`'s `call` arm
(`src/check/poly.rs`, `~:1275`), where the `top.quot`-is-`None` branch currently tests only
`PolyType::Concrete(Type::Quotation(eff))`, add a second match arm for
`PolyType::Quotation(ins, outs, ..)`, matched **unconditionally** — no `is_inline`/row guard.
The scope note above is why: a non-combinator word can never carry `is_inline = true` or a
row-carrying effect on any of its declared quotation slots (rejected at parse time), so an
extra condition here would exclude a shape that cannot arrive, adding an untestable branch
for no coverage. Every non-quotation operand keeps falling through to
`poly_op_on_variable_error`, unchanged. Because `ins`/`outs` are `Vec<PolyType>` (not `Copy`,
unlike the ground arm's `Type` payload) and `top` is a shared reference (`stack.last()`),
the new arm matches on `&top.pt` (or clones it first) rather than moving `top.pt` by value —
the existing `if let PolyType::Concrete(Type::Quotation(eff)) = top.pt` pattern just above it
only compiles because `Type` is `Copy`; the same syntax will not compile unchanged for
`PolyType::Quotation`'s non-`Copy` fields. The ground `Concrete(Type::Quotation)` arm and the
literal `top.quot` arm are untouched.

**R2: consume and produce by structural row comparison, variables rigid.**
`poly_call_abstract_quotation_param` is the abstract twin of
`poly_call_ground_quotation_param`: it consumes `ins.len()` operands deepest-first and pushes
`outs`. Because the body is checked once, generically, with every variable rigid
(`check_poly_body` builds no `Subst` mid-body), each declared row slot is compared
**structurally** against the operand's own `PolyType` via `PolyType`'s derived `Eq` (the same
absence-of-`Subst`, structural-comparison discipline S3b's L1 already established for
`poly_eliminator_call`'s arm exit rows). A declared input equal (structurally) to the
operand beneath it consumes it; anything else is a located mismatch. No `Subst` is built or
consulted.

**R3: located under/mismatch diagnostics, mirroring the ground arm.** An underflow (fewer
operands than `ins.len()`) reports through the existing `underflow_error`; an operand whose
`PolyType` is not structurally equal to the declared input reports through the existing
`poly_rendered_type_mismatch_error`, rendering both the declared slot and the found operand
through `poly_type_str` (both sides are `PolyType`, so neither has a bare `Type` to hand the
two-`Type` renderer). This replaces the single blanket "not permitted on a quotation" message
for every `PolyType::Quotation` operand reaching this arm — per R1, that is now every
`PolyType::Quotation` a non-combinator word can ever own, so the blanket message's only
remaining source is a genuinely non-quotation operand (unchanged, `poly_op_on_variable_error`).

**R4: lowering is confirmed, not assumed.** By IR construction the body is monomorphized per
concrete instantiation, so the `[ 'T -- 'T ]` parameter has already grounded to a `Type::
Quotation` via `apply_subst`'s `Quotation` arm, and the existing indirect-call path
(`src/ir/func_builder/quotation.rs`, `lower_indirect_call`, `~:205`), already exercised
end-to-end by the ground-parameter case, should carry this shape with no change. **Phase 1
confirms this against live source before phase 2 assumes it**; if a representation gap exists,
phase 2 adds the minimal lowering arm and the spec's exit findings record it. No new lowering
machinery is planned.

## Tests

**Unit, beside the checker stage (`src/check/poly.rs`).**

- `poly_call_on_an_abstract_quotation_param_is_still_error` (`~:7349`) is **flipped** from a
  rejection assertion to an accept case: the same `[ i64 -- 'T ]`-shaped source now checks
  clean. Renamed to `thing_condition_expected` form
  (`poly_call_on_an_abstract_quotation_param_is_accepted`).
- The underflow arm (R3): a declared input with nothing beneath the quotation reports
  `` `call` needs N values, but the stack holds M`` (parallel to the ground arm's existing
  `poly_call_on_a_ground_quotation_param_underflow_is_error`).
- The type-mismatch arm (R3): an operand whose `PolyType` is not structurally equal to the
  declared input reports through `poly_rendered_type_mismatch_error`, both sides rendered by
  `poly_type_str`.
- `poly_call_on_a_variable_local_is_still_error` stays green (a bare `'T` local is not a
  quotation and keeps its own rejection).
- No test targets a `~`-declared or row-carrying quotation operand reaching this arm: per the
  Scope section, no non-combinator word can ever construct one, through the parser or
  otherwise relevant to this call site, so there is no witness program and none is owed.

**Golden, end-to-end (`tests/phase7_slice3f.rs`).**

- `body_boundary_rejects_an_abstract_quotation_param` (`~:170`) is **flipped** to an accept
  case: the `apply`-shaped source builds and runs. Renamed accordingly
  (`body_boundary_accepts_an_abstract_quotation_param`).
- The headline golden: `: apply ( 'T [ 'T -- 'T ] -- 'T ) call ;` with
  `4 [ 1 add ] apply print` builds, runs, and prints `5`, confirming R4's lowering path
  end-to-end. A multi-instantiation golden (the same `apply` used at two concrete `'T`) is
  included only if phase 1's recon shows it exercises a distinct lowering path; otherwise it
  is redundant with S3f's ground goldens and omitted.

Both flipped tests are cited here so the pipeline does not read their red diff as a
regression.

## Phase plan

Two phases; the surface is a single dispatch arm plus a new function that mirrors an existing
one, so it does not split further.

| Phase | Focus | Effort | Difficulty |
| --- | --- | --- | --- |
| 1 | Recon the lowering path, add the checker accept arm and its diagnostics, flip both pinned unit assertions | S | standard |
| 2 | End-to-end goldens (build and run), any minimal lowering arm the recon requires, green gate | S | standard |

```json
{
  "phases": [
    { "phase": 1, "focus": "checker accept arm for an abstract quotation parameter, its located under/mismatch diagnostics, and flipping the pinned unit assertions", "effort": "S", "difficulty": "standard" },
    { "phase": 2, "focus": "end-to-end build-and-run goldens, any minimal lowering arm recon requires, and the green gate", "effort": "S", "difficulty": "standard" }
  ]
}
```

## Exit criteria

1. `: apply ( 'T [ 'T -- 'T ] -- 'T ) call ;` with `4 [ 1 add ] apply print` builds, runs,
   and prints `5`.
2. A body `call` whose stack holds fewer operands than the declared quotation's inputs is a
   located underflow (`` `call` needs N values, but the stack holds M``), not the blanket
   message.
3. A body `call` on an operand whose `PolyType` is not structurally equal to the declared
   input is a located mismatch rendered through `poly_type_str` on both sides.
4. Both formerly-pinned rejection tests (`src/check/poly.rs`,
   `tests/phase7_slice3f.rs`) now assert acceptance.
5. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green at each phase exit.
