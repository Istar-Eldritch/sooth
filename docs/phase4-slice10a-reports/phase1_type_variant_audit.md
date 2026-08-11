# Phase 1 — the type variant and the audit

Slice 10a, phase 1 of 7. Delivers **R1 partial**, **R2 partial**, **R10 partial**.

Base commit this slice branches from (R19, for the phase-7 baseline check):
**`dbdb4a3`** (`feat(tooling): add tree-sitter grammar for syntax highlighting`).

## What landed

- `Type::InlineQuotation(&'static QuotEffect)` beside `Type::Quotation`
  (`src/ast.rs`). `Type` stays `Copy` (the payload is `&'static`) and structural
  `PartialEq`, so `InlineQuotation(e) != Quotation(e)` at every equality site —
  R3 is free.
- The three exhaustive matches the compiler flagged, filled: `Type::name()`
  (`src/ast.rs`, the `~[ ... ]` spelling, R10's phase-1 half), `type_node()`
  (`src/check.rs`, `None` — a `~` is never a field), and `ir_type_of()`
  (`src/ir.rs`, `unreachable!()` — a `~` never reaches the backend). No fourth:
  `cargo check --all-targets` compiled clean immediately after filling these
  three, matching the spec's "three errors" measurement.
- `inline_quotation_type(inputs, outputs) -> Type` constructor: mirrors
  `quotation_type` but leaks a `~`-prefixed spelling into `name_static`, so the
  two variants never share a `&'static QuotEffect` and render distinctly.
- `is_quotation_type(Type) -> Option<&'static QuotEffect>` accessor
  (`src/ast.rs`): `Some` for both quotation variants, and every **enabling** and
  **routing** site routes through it. Deliberately **not** used at the four
  materialization boundaries (they reject a `~` by type inequality); **is** used
  at the declaration-position rejections and the capture-admission guard (which
  fail open otherwise).

Unit tests (constructed directly, no parser — this is what makes phase 1
testable in isolation): `ast::tests::{inline_quotation_type_name_carries_the_tilde,
is_quotation_type_accepts_both_variants_only, inline_and_ordinary_quotation_are_never_equal}`
and `check::tests::{poly_input_is_quotation_recognizes_inline,
word_declares_quotation_parameter_recognizes_inline,
reject_quotation_type_position_rejects_inline,
audit_rejects_inline_quotation_output_but_allows_ordinary}`.

Not in phase 1 (named here so the audit's "leave alone" lines are unambiguous):
the `~[` token and the four parse entry points (phase 2), the row fields on
`PolyType::Quotation` and the `apply_subst`/fold grounding of `~` (phases 2–3),
the row-aware poly renderers (phase 3), row grounding at the check sites
(phase 4), the back-edge rewrite (phase 5). No `~` is producible from source
yet, so nothing here is user-visible; a missed site would fail **silently**,
which is why the audit below is the deliverable.

## The audit

Pinned to the spec's grep, run against the tree after phase 1's edits:

```
grep -n 'Type::Quotation' src/check.rs src/ir.rs | grep -vE '://|/// '
```

One disposition per matched line. Tags:

- **[exhaustive]** — one of the three arms the compiler forced; filled.
- **[enabling]** — extended via the accessor so a `~` is accepted like an
  ordinary quotation; breaks loudly if missed.
- **[routing]** — routing predicate / let-else gate; extended via the accessor
  so a `~` is classified as a quotation parameter (missing it makes the word not
  a combinator → lowered as a call → `ir_type_of` `unreachable!`).
- **[fail-open→fixed]** — a materializing declaration that returned `Ok` for a
  `~`; now rejects both variants via the accessor.
- **[leave-narrow]** — stays `Type::Quotation`-only; a `~` is unreachable here
  (banned upstream) and narrowness is the safe default (no materialization).
- **[poly-layer]** — a `PolyType::Quotation` arm; the `~` flag/row on the poly
  layer is a later phase (named). Untouched by phase 1.
- **[comment]** / **[test]** — prose or a test body; no behaviour.
- **[backend]** — an `IrType::Quotation` site; a `~` never reaches `IrType`
  (`ir_type_of` `unreachable!`), so it cannot arrive here. Untouched.

### `src/check.rs`

```
1873  if matches!(fty, Type::Quotation(_)) {          [leave-narrow] struct-field allow-guard: an ordinary quotation field is legal (continue); a `~` does NOT continue and falls to reject_quotation_type_position (fixed at 2040).
1899  if matches!(a.element, Type::Quotation(_)) {     [leave-narrow] array-element allow-guard: same as 1873 — `~` falls through to the fixed reject.
1921  // R8 (D4): ... Type::Quotation output              [comment]
1925  if matches!(slot.ty, Type::Quotation(_)) {        [leave-narrow] word-output allow-guard: an ordinary quotation output is a materialization boundary (legal); a `~` does NOT continue and hits the fixed reject.
1988  PolyType::Quotation(ins, outs) =>                  [poly-layer] audit_poly_input_quotation; row/`~` flag is phase 3.
2016  PolyType::Quotation(..) => Err(...)               [poly-layer] reject_poly_quotation_anywhere; phase 3.
2036  // `Type::Quotation` reaches this only after ...     [comment] (on the fixed reject).
2040  if let Some(eff) = is_quotation_type(ty) {        [fail-open→fixed] reject_quotation_type_position: was `if let Type::Quotation`, returning Ok for a `~`. Now rejects both. The single fix that makes 1873/1899/1925 and the cell/ref-referent positions reject a `~`. Mutation-tested (below).
2185  // ... `Type::Quotation` input qualifies.            [comment] collect_combinators doc. (R9 note: the "monomorphic only" claim here is stale; its correction is phase 4, per the spec.)
2588  PolyType::Quotation(ins, outs) =>                  [poly-layer] phase 3.
3346  | Type::Quotation(_) | Type::InlineQuotation(_)   [exhaustive] type_node(): both → None (a `~` is never a value-containment field).
4494  // R7/R15/D4: a declared `Type::Quotation` output   [comment]
4500  if let Type::Quotation(eff) = *want {              [leave-narrow] materialization boundary #1 (word output). A `~` output is banned at declaration (audit_word_quotation_positions), so `want` is never a `~`; and `~` != `Type::Quotation` means no materialization even if it were. Rejects by inequality before the boundary.
4772  PolyType::Quotation(..) => true,                   [poly-layer] contains_poly_quotation-style; phase 3.
5430  PolyType::Quotation(..) =>                          [poly-layer] phase 3.
5707  ...matches!(want, Type::Quotation(_)) || s.ty==... [leave-narrow] resolve_combinator_overload, mono arm: skips exact-type-eq for a quotation param slot. A mono `~` param is impossible (`~` is poly-forced), so unreachable; narrow is safe.
5923  PolyType::Quotation(ins, outs) =>                  [poly-layer] unify_poly_input arm; the row exclusion (R8) is phase 3.
6002  // yielding a concrete `Type::Quotation`.            [comment]
6003  PolyType::Quotation(ins, outs) =>                  [poly-layer] apply_subst arm; `~` grounds to InlineQuotation here in **phase 2**.
6152  PolyType::Quotation(..) => "a quotation"            [poly-layer] poly_type_str-adjacent renderer; row-aware in phase 3.
6309  PolyType::Quotation(ins, outs) =>                  [poly-layer] poly_type_str; row-aware in phase 3.
6961  PolyType::Quotation(..) => true,                   [routing] poly_input_is_quotation: `PolyType::Quotation` → true; the `Concrete(t)` sub-arm now routes through the accessor so a fully-concrete `~` (`Concrete(InlineQuotation)`) is recognized as a quotation parameter. Unit-tested.
7138  // combinator (a `Type::Quotation` slot ...)         [comment] (inside inline_combinator's abstract-forward arm, now accessor-gated at its head).
7289  // ... grounds ... to `Type::Quotation` and (ph2)    [comment] (on the splice let-else, now accessor-routed).
7612  // or an already-erased `Type::Quotation`) ...        [comment] (on the capture-admission guard, fixed at its head).
7703  ..Slot::computed(Type::Quotation(eff))             [leave-narrow] materialize_quotation_at_boundary producer: materialization always yields a runtime `Type::Quotation`, never a `~`. Correct as-is; a `~` never reaches here (all boundaries reject it first).
8187  // `Slot.ty == Type::Quotation`, no `Known` ...       [comment] (in `call`, now accessor-gated).
8372  // R8 (D4): `!`/`+!` into a `&!Type::Quotation` ...   [comment]
8388  if let Some((Type::Quotation(eff), _)) = ref_parts [leave-narrow] materialization boundary #2 (`&!` store). A `~` referent is banned (reference-referent field rejection at 2040), so unreachable; narrow means a stray `~` referent would fall through without materializing — the safe default.
8508  // `Type::Quotation`); both are dropped ...           [comment] (on the back-edge filter, extended below).
8542  ...s.quot.is_none() && is_quotation_type(s.ty).is_none()  [fail-open→fixed / back-edge filter] extended so a `~` abstract-param slot is dropped from the carried state, not carried across the edge. (Phase 5 deletes this line when it rewrites the arm to ground declared outputs.)
8575  // R8 (D4): a declared `Type::Quotation` parameter    [comment]
8585  // a `Type::Quotation`, rejected at declaration).     [comment]
8586  if let Type::Quotation(eff) = *want {              [leave-narrow] materialization boundary #3 (declared parameter). A `~` parameter is poly-forced, so this mono site never sees one; `~` != `Type::Quotation` rejects by inequality otherwise.
8639  (ty.is_aggregate() || matches!(ty,Type::Quotation)) [leave-narrow] surviving-set forward onto a quotation-typed output. A `~` output is banned, so `ty` is never a `~` here.
8807  .filter(|t| matches!(t, Type::Quotation(_)))       [leave-narrow] `if`-join erasure #4a: filters the `&!` referent to a quotation. `expected` is sourced only from a declared output or a `&!` referent, both banned for a `~` at declaration — unreachable by construction.
8810  Some(Type::Quotation(eff)) =>                       [leave-narrow] materialization boundary #4 (`if`-join). Unreachable for a `~` for the same reason as 8807.
8830  // real `Type::Quotation`, no `Known` marker.         [comment]
8831  (None, Some(Type::Quotation(eff)), merged_set)     [leave-narrow] `if`-join producer: emits a runtime `Type::Quotation`, never a `~`.
14971 // `PolyType::Quotation` arm directly ...             [comment] (test prose).
14976 inputs: vec![PolyType::Quotation(...)]             [test] existing poly-combinator test fixture.
15045 // ... Deleting the `Type::Quotation` clause ...      [comment] (test prose).
```

The fifth materialization boundary — capture admission — is not on a
`Type::Quotation`-spelled line in this grep because it now reads
`is_quotation_type(b.ty)`; it is dispositioned explicitly:
**`check_capture_admission` guard [fail-open→fixed]** — was
`b.quot.is_some() || matches!(b.ty, Type::Quotation(_))`, which passed a `~`
local straight into a surviving capture set (the one thing `~` forbids). Now
accessor-gated. Phase 2 swaps the shared "deferred" message for a `~`-specific
"a `~` quotation cannot be captured" located error plus its golden.

Two more accessor-routed sites carry no `Type::Quotation`-spelled line after the
edit (routed via the accessor), dispositioned explicitly:

- **`word_declares_quotation_parameter` mono branch [routing]** — a mono word
  with a `~` input now counts as declaring a quotation parameter, so it is
  inlined, not lowered to a call. Unit-tested.
- **`audit_word_quotation_positions` input/clause-body walks [fail-open→fixed]**
  — the `.any(...)` combinator-detect and the `if let ... = slot.ty` main/nested
  walk now see a `~` (accessor), so `main`-takes-`~` and quotations nested in a
  `~` effect are rejected.
- **`inline_combinator` mono gate + abstract-forward [routing/enabling]**,
  **`check_poly_combinator_args` splice let-else + abstract-forward
  [routing/enabling]**, **`unify_poly_input` let-else [routing]**, **`call`
  abstract [enabling]**, **`times` abstract [enabling]** — all accessor-routed.
  The two let-elses no longer become a spurious `unreachable!` once phase 2/4
  grounds a `~` parameter to `Type::InlineQuotation`.

### `src/ir.rs`

```
256   // no interning table threaded ...                    [comment]
259   Type::Quotation(eff) => IrType::Quotation(...)      [exhaustive-adjacent] ir_type_of ordinary arm, unchanged.
262   // upstream (it is not `Type::Quotation`) ...          [comment] (on the new InlineQuotation `unreachable!` arm).
821   Type::Quotation(_) => { ... quotation_layout ... }  [leave-narrow] size_align: a quotation field's layout. `~` hits the `_ =>` fallthrough → `ir_type_of(~)` → `unreachable!` — correct, a `~` is never a field so size_align is never asked for one.
569,588,1445,1472,2127,2513,2642,3955,3974,3987,4075,4098,4166,4168,4477,4499,4525,4563,4648,4653,4688,8902,8913,8921,8922
                                                          [backend] every remaining line is an `IrType::Quotation` (or a `PolyType::Quotation` test fixture at 8902). A `~` is `Type::InlineQuotation`, never lowered to any `IrType`, so it cannot reach any of these. Untouched.
2279  PolyType::Quotation(..) =>                          [poly-layer] mangling-side poly arm; no `~` grounding impact (R9: no mangling change).
```

## R20 — mutation evidence for phase 1's new guards

Phase 1 introduces one class of *located* guard: the declaration-position
rejection of a `~` (`reject_quotation_type_position`, and through it the
struct-field / array-element / word-output / cell / ref-referent positions).

Mutation: revert `reject_quotation_type_position`'s head from
`if let Some(eff) = crate::ast::is_quotation_type(ty)` back to the fail-open
`if let Type::Quotation(eff) = ty`. Result:

```
test check::tests::reject_quotation_type_position_rejects_inline ............ FAILED
test check::tests::audit_rejects_inline_quotation_output_but_allows_ordinary  FAILED
test result: FAILED. 768 passed; 2 failed
```

Both flip, then pass again once the guard is restored — the tests are not
placebos. The routing predicates (`poly_input_is_quotation`,
`word_declares_quotation_parameter`) are pinned by
`poly_input_is_quotation_recognizes_inline` and
`word_declares_quotation_parameter_recognizes_inline`; their user-visible
consequence (a `~` word inlined rather than reaching `ir_type_of`'s
`unreachable!`) has no source-producible `~` yet, so it is proven end-to-end in
phase 2 and audited in phase 7.

## Green

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
all pass (770 lib tests, full suite green).
