# Phase 7 Slice 3l: a poly body may `call` its own abstract quotation parameter

## Goal

A plain (non-`inline`, non-combinator) polymorphic word may now `call` a quotation-typed
parameter it declared, from inside its own body, when that parameter's brackets still
mention the word's own type variables (`[ 'T -- 'T ]`). This slice closes the body side only.

**Correction (phase 1 recon finding, review round 1).** The claim that "the call-site side of
this shape already works" is false: `check_poly_call`'s R9p guard materializes a *literal*
quotation argument only against a **ground** `Concrete(Type::Quotation(eff))` declared
position; an abstract `PolyType::Quotation` slot falls to `reject_quotation_argument`
(`src/check/poly.rs:4518`), so `unify_poly_input`'s `Quotation` arm is never reached from a
literal call-site argument. Probe-verified live: the headline program below is rejected at
`main`'s call site, not accepted, with
`` error: a quotation cannot be passed to `apply`; only `call` accepts one in `main` ``.
Closing that gap (if it is done at all) is phase 2's concern; see R4 and the phase table
below. The headline shape *does* build and run when the quotation argument reaches `apply`
by some route other than a literal at that call site (e.g. forwarded out of another word's
return value) — `tests/phase7_slice3f.rs`'s `body_boundary_calls_an_abstract_quotation_param`
is the phase 1 golden for that shape. The literal-at-call-site form was closed in phase 2
(below): `check_poly_call`'s R9p guard now grounds an abstract declared quotation slot
through a completed `Subst` before materializing the literal, order-independent of the
declared parameter position (two-pass, mirroring `check_poly_combinator_args`):

```
import: intrinsics * ;
: apply ( 'T [ 'T -- 'T ] -- 'T ) call ;
: main ( -- ) 4 [ 1 add ] apply . ;
```

(`print` is not a Sooth word; the print intrinsic is `.`, imported per the line above —
verified live: `print` alone reports `` unknown word `print` ``.)

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
  this arm at all, on any word: `check_inline_quotation_requires_inline`
  (`src/check/word_entry.rs:122-144`, called from `check.rs:735` and `repl.rs:3060`) rejects
  a `~[ ]` parameter at declaration unless the word also declares `inline` (`is_combinator`,
  `src/check/combinators.rs:155`, is merely the one-line `word.declares_inline` predicate
  that rejection is gated on, not the rejection itself), and
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
end-to-end by the ground-parameter case, should carry this shape with no change once the
parameter's own declared row is grounded. **Phase 1 confirms this against live source before
phase 2 assumes it**; a representation gap did exist, and phase 1 closed it (below), since
leaving it open would have shipped a checker accept arm whose only realistic reachable
program crashed the compiler.

**Recon finding, closed in phase 1 (review round 1 blocker).** `subst_polytype`
(`src/ir/driver.rs`), the lowering-side twin of check's `apply_subst`, had a
`PolyType::Quotation(..) => unreachable!(...)` arm whose comment asserted "a quotation-taking
word is never monomorphized to a standalone `IrFunc`". That premise was already false before
this slice for *any* non-combinator word declaring an abstract quotation parameter, called
with a concrete instantiation, regardless of whether its body ever `call`s that parameter
(probe-verified: a body that only `drop`s such a parameter hit the same `unreachable!` at
`HEAD~1`, unrelated to this slice's `call` arm). R1's new `call` arm added a second,
checker-visible path to the same pre-existing gap, turning what used to be a clean checker
rejection (`` `call` is not permitted on a quotation ``) into a lowering-time panic for the
`call`-using shape specifically. Fixed in phase 1 by mirroring `apply_subst`'s `Quotation`
arm: substitute both rows through `θ`, then ground to `Type::Quotation`/`Type::
InlineQuotation` via `crate::ast::quotation_type`/`inline_quotation_type` (no interning
needed — those constructors mint a fresh leaked effect per call, unlike the `Array`/`Ref`
arms' lookup-only style). This is a lowering **bug fix**, not new machinery: it makes
`subst_polytype` agree with what `apply_subst` already computed on the check side for the
same shape.

**Recon finding, left open for phase 2.** The R9p call-site gap (Goal section correction,
above) is a separate, pre-existing gap at the *argument* boundary, not the body boundary this
slice's R1–R3 touch. It is out of phase 1's scope: `check_poly_call`'s materialization guard
is a different function on a different call path (`poly_call_term` is never involved), and
closing it is a call-site unification change, not a body-`call` dispatch change. Recorded here
so phase 2 does not understate it as an afterthought.

## Tests

**Unit, beside the checker stage (`src/check/poly.rs`).**

- `poly_call_on_an_abstract_quotation_param_is_still_error` (`~:7349`) is **flipped** from a
  rejection assertion to an accept case: the same `[ i64 -- 'T ]`-shaped source now checks
  clean. Renamed to `thing_condition_expected` form
  (`poly_call_on_an_abstract_quotation_param_is_accepted`).
- New: `poly_call_on_an_abstract_quotation_param_underflow_is_error` — the underflow arm
  (R3), a declared input with nothing beneath the quotation, reports
  `` `call` needs N values, but the stack holds M`` (parallel to the ground arm's existing
  `poly_call_on_a_ground_quotation_param_underflow_is_error`).
- New: `poly_call_on_an_abstract_quotation_param_mismatch_is_error` — the type-mismatch arm
  (R3), an operand whose `PolyType` is not structurally equal to the declared input, reports
  through `poly_rendered_type_mismatch_error`, both sides rendered by `poly_type_str`.
- `poly_call_on_a_variable_local_is_still_error` stays green (a bare `'T` local is not a
  quotation and keeps its own rejection).
- **Added (review round 1, coverage gap).** Neither of the accept-case tests above declares
  more than one input or one output, so neither can discriminate a reversed consumption or
  push order from the correct one — the ground twin
  (`poly_call_on_a_ground_quotation_param_pushes_outputs_in_order`,
  `body_boundary_pops_declared_inputs_deepest_first`) pins this for
  `poly_call_ground_quotation_param` only, not the new abstract arm.
  `poly_call_on_an_abstract_quotation_param_pops_declared_inputs_deepest_first` and
  `poly_call_on_an_abstract_quotation_param_pushes_outputs_in_order` each declare two
  heterogeneous slots and are mutation-tested: reversing either loop in
  `poly_call_abstract_quotation_param` turns its own test red.
- No test targets a `~`-declared or row-carrying quotation operand reaching this arm: per the
  Scope section, no non-combinator word can ever construct one, through the parser or
  otherwise relevant to this call site, so there is no witness program and none is owed.

**Golden, end-to-end (`tests/phase7_slice3f.rs`).**

- `body_boundary_rejects_an_abstract_quotation_param` (`~:170`) is **replaced**, not
  mechanically flipped: its current source (`` : call_it ( 'T: Copy [ i64 -- 'T ] -- 'T ) ``
  called as `` [ 5 ] call_it drop ``) supplies only the quotation operand to a two-input
  word, arity-invalid on its own terms — today it never reaches an arity check because the
  body-internal rejection fires first, but once that rejection is lifted the same source
  would fail on a *different*, unrelated arity error at the `call_it` call site, not build
  clean. Verified live: `call_it` declares two inputs (`'T` and the quotation), `main`
  pushes only one.
- **Correction (review round 1).** At phase 1 exit, the `apply` shape could not be built as
  a real golden through a *literal* call-site argument (`4 [ 1 add ] apply .`): the
  Goal-section correction above meant that source hit the R9p rejection, not a clean build,
  so phase 1 could not replace the original check-only assertion with a `build_and_run` one
  as first planned. Its replacement, `body_boundary_calls_an_abstract_quotation_param`,
  forwards the quotation argument out of a helper word's return value instead (`mk_i64`/
  `mk_bool`), which does not touch R9p, and is a full `build_and_run` golden run at two
  instantiations of `'T` — not a `check_ok` placebo. It also depends on, and exercises, the
  `subst_polytype` lowering fix (R4) alongside the checker accept arm, so it was the one
  golden that proved the whole phase 1 path end-to-end, headline call-site literal aside.
- **Phase 2.** `headline_apply_accepts_a_literal_quotation_argument` is the literal
  call-site golden the round-1 correction above could not yet build, now closed by the R9p
  fix; it runs two instantiations of `'T` (`i64` and `Bool`), same discipline as the phase 1
  golden above. `check_poly_call_materializes_an_abstract_quotation_argument_declared_first`
  (`src/check/poly.rs`) pins the two-pass reorder itself: the declared quotation slot before
  the plain input that binds `'T`, the reverse of the phase 2 unit test alongside it.

The replaced/renamed tests are cited here so the pipeline does not read their diff as a
regression.

## Phase plan

Two phases; the surface is a single dispatch arm plus a new function that mirrors an existing
one, so it does not split further.

| Phase | Focus | Effort | Difficulty |
| --- | --- | --- | --- |
| 1 | Recon the lowering path, add the checker accept arm and its diagnostics, flip both pinned unit assertions, fix the `subst_polytype` lowering gap the recon found | S | standard |
| 2 | Close the R9p call-site gap so a literal quotation argument reaches an abstract declared position, then the headline `apply` golden (build and run), green gate | S | standard |

```json
{
  "phases": [
    { "phase": 1, "focus": "checker accept arm for an abstract quotation parameter, its located under/mismatch diagnostics, flipping the pinned unit assertions, and the subst_polytype lowering fix the recon found", "effort": "S", "difficulty": "standard" },
    { "phase": 2, "focus": "close the R9p call-site materialization gap for an abstract quotation position, then the headline build-and-run golden and the green gate", "effort": "S", "difficulty": "standard" }
  ]
}
```

## Exit criteria

1. `import: intrinsics * ;` then `: apply ( 'T [ 'T -- 'T ] -- 'T ) call ;` with
   `4 [ 1 add ] apply .` builds, runs, and prints `5\n`. **Not met at phase 1 exit** — the
   R9p call-site gap (Goal section correction) rejected this exact source at `main`'s
   call site; phase 1's `body_boundary_calls_an_abstract_quotation_param` golden proved the
   same body-boundary shape end-to-end via a non-literal argument instead. **Met at phase 2
   exit**: `headline_apply_accepts_a_literal_quotation_argument`
   (`tests/phase7_slice3f.rs`) builds and runs this exact source.
2. A body `call` whose stack holds fewer operands than the declared quotation's inputs is a
   located underflow (`` `call` needs N values, but the stack holds M``), not the blanket
   message. Met at phase 1 exit.
3. A body `call` on an operand whose `PolyType` is not structurally equal to the declared
   input is a located mismatch rendered through `poly_type_str` on both sides. Met at phase
   1 exit.
4. Both formerly-pinned rejection tests (`poly_call_on_an_abstract_quotation_param_is_accepted`
   in `src/check/poly.rs`, `body_boundary_calls_an_abstract_quotation_param` in
   `tests/phase7_slice3f.rs`) now assert acceptance. Met at phase 1 exit.
5. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green at each phase exit.

**Note on roadmap wording.** `docs/roadmap/P7-language-prereqs.md`'s S3l entry describes the
body popping/pushing "against the row grounded through that call's `Subst`." R2 is the
correct mechanism: no `Subst` is built or consulted mid-body (variables stay rigid; rows are
compared structurally via `PolyType`'s derived `Eq`, S3b's L1 discipline) — S3f's own ground
arm this slice mirrors builds no `Subst` either, so the roadmap's phrasing was already loose
before this spec. This spec's R1–R4 are authoritative; the roadmap entry should be corrected
to match at exit, not the reverse.
