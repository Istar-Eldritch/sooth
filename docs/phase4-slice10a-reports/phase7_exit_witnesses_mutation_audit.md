# Phase 7 — exit witnesses and mutation audit

Slice 10a, phase 7 of 7 (final). Delivers **R15–R20**.

> **No separate phase-5 report exists.** Phase 5 (`feat(phase-5)`, `abb934c` on this branch
> post-rebase) landed with no standalone report, unlike phases 1–4, 6, and 7. Its exit evidence
> (R12's mutation audit) is gathered retroactively and folded into this report's R20 section
> below, under "Phase 5".

## What landed

`tests/phase4_slice10a_exit_witnesses.rs`, seven tests covering R15–R19, plus
this report's R20 audit consolidating every located error introduced across
phases 1–6, one row per guard, each with mutation evidence.

## R15 — `my-times` compiles beside the untouched intrinsic, sums, constant stack

`my_times_compiles_beside_the_untouched_intrinsic_and_sums`: the recon-4
`my-times` (`..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s`, `from`/`to` as the two
`i64` inputs, per the spec's correction of the brief) and the untouched
intrinsic `times` are called from the same `main`, printing `10` (0+1+2+3+4)
and `6` ((0+1)+(1+1)+(2+1)). Neither shape changed the other.

`my_times_runs_one_million_iterations_in_constant_stack`: `0 0 1000000 [ drop
1 + ] my-times` under `ulimit -s 1024` (the same witness shape as
`three_deep_times_nesting_runs_in_constant_stack`,
`tests/phase4_combinators.rs:1403`), exits 0 printing `1000000`. A
per-iteration `Call` would overflow this stack well before 1M rounds; TCO on
the self-tail back-edge is what a `~`-typed user word buys for free, per the
spec's opening claim ("everything under `times` is already general").

## R16 — grounding is type-only; a borrow can be substituted

`row_grounding_accepts_a_borrow_of_an_unrelated_place_of_the_same_type`: a
non-recursive, single-call combinator (`apply-with-v`, R9 context 1 shape) is
given an incoming row-borrow of `a` (value 0) and a fixed-parameter borrow of
`b` (value 9); its body `[ swap drop ]` drops the row's `a`-borrow and leaves
`b`'s as the new row. The declared row type (`&!V`) matches on both sides, so
`match_slot`'s `Exact` path accepts it — `Slot::computed`'s dropped `deriv`
means nothing in the grounding mechanism could object even if it wanted to.
The printed field is `9`, `b`'s, not `0`, `a`'s — proving the substitution
actually took effect at runtime, not merely that the checker let the program
through. Pinned by value, per R16's instruction not to settle for "merely a
value swap": this is a *different referent* of the same type, not the same
place holding a new value.

(A same-referent, real self-tail back-edge attempt — swapping two live
`&!V`s across `my-times`'s own `if`/`else` — was tried first and correctly
rejected by the *general* `if`-join borrow-consistency check, unrelated to
row grounding: both arms of a real back-edge must agree on which place stays
borrowed, a pre-existing invariant this slice does not touch. R16's claim is
about the *type-only* row-prepend at the literal-check boundary specifically,
so the witness is built at that boundary, not by fighting an orthogonal
check.)

## R17 — aggregate carried across the row, per-iteration data dependence

`my_times_carries_an_aggregate_without_aliasing`: `Acc { x i64, y i64 }`
rides `my-times`'s row across 5 iterations; each iteration reads the
*previous* iteration's `x`/`y` to compute the next (`new_x = x0 + i`, `new_y
= y0 + x0 + i`), so a stale or aliased blit of the carried struct (the
slice-3 aliasing class fixed by the stable-slot mechanism,
[[project_aggregate_return_aliasing]]) would surface as an arithmetically
wrong number, not a crash. Hand-traced `(x, y)`: `(0,0) → (0,0) → (1,1) →
(3,4) → (6,10) → (10,20)`; the program prints `20` then `10` (`Acc>` pushes
`x` under `y`, `. .` prints top-first). Matches.

## R18 — nesting parity

`my_times_nested_in_itself_produces_correct_output`: the outer `my-times` (3
rounds) sums an inner `my-times` (2 rounds, `+1` each round, total 2) into
its own row each outer round: `2+2+2 = 6`. The inner call's `stack[..base]`
picks up the outer's own row sitting underneath it on the shared stack, so
row grounding composes under nesting with no extra plumbing — the same
`stack[..base]` slicing that grounds a single row also grounds a nested one,
because "the row" is simply defined as "whatever's below the fixed inputs",
recursively.

(First attempt hit two self-inflicted bugs during construction, not compiler
defects: a doubled index-consumption — `| j | drop 1 +` binds the index via
`| j |` *and* drops it again, underflowing `+` — is a shape identical to what
`[ drop 1 + ]`'s *unnamed* form already relies on for the single-drop case
[[project_rows_in_quotation_effects_blocker]]-adjacent; and a `&!V` fixed
parameter threaded through a self-tail loop and reused both inside the body
and in the recursive forwarding call is rejected as a reborrow conflict,
correctly — a non-`Copy` value can't be used twice in one recursive
definition regardless of runtime `N`, which is why R16's witness above uses a
non-recursive combinator instead. Neither is a slice defect; both are
recorded here because they cost real debugging time and the next person
hitting `~[` should not re-derive them.)

## R19 — no regression

- `combinators_library_contains_no_tilde` (review fix, cycle 3: dropped the
  git-show byte-comparison this test originally also ran against
  `dbdb4a3:lib/combinators.sth`, the base commit this slice branches from,
  named in phase 1's report -- it broke in a non-git build tree, a shallow
  clone, or once the pinned SHA became unreachable; byte-identity is already
  covered, without a hardcoded SHA, by `tests/qbe_baseline.rs`'s
  `corpus_qbe_stays_byte_identical_to_baseline`): the working tree's copy of
  `lib/combinators.sth` grepped for `~` — absent.
- `while_is_unaffected_by_the_row_and_back_edge_rewrite`: the value-level twin
  of `while_self_tail_still_checks_after_back_edge_rewrite` (`src/check.rs`),
  run end to end (`0 [ dup 5 < if 1 + true else false end ] while` → `5`).
- The corpus/QBE-baseline half of R19 is `tests/qbe_baseline.rs`'s existing
  `corpus_qbe_stays_byte_identical_to_baseline`, unaffected by this phase (no
  corpus file touched) and confirmed still green.
- No index type changed anywhere in this slice (spec's explicit
  constraint); the intrinsic's two hardcoded `Type::I64` sites in
  `check_abstract_quotation_times` (`src/check.rs:7043`, `:7046`) and the
  Known-literal count check in the `times` term-checking arm (`:8646`) are
  untouched — `my_times_compiles_beside_the_untouched_intrinsic_and_sums` is
  the end-to-end confirmation.

## R20 — the mutation audit

Per-phase, every located error this slice introduced, its guard, the
mutation applied, and the test(s) it flips. Phases 1–4 and 6 already recorded
this in their own reports; reproduced here as the single enumeration the
spec asks for, plus phase 5's evidence (R12), gathered fresh in this phase
since phase 5 landed no report of its own.

### Phase 1 — R2 (declaration-position rejection)

| Guard | Mutation | Flips |
| --- | --- | --- |
| `reject_quotation_type_position`'s accessor gate | revert to `if let Type::Quotation(eff) = ty` (fail-open) | `reject_quotation_type_position_rejects_inline`, `audit_rejects_inline_quotation_output_but_allows_ordinary` (768 passed, 2 failed) |

### Phase 2 — R1 (syntax), R2 (materialization + capture), R3, R6

| Guard | Mutation | Flips |
| --- | --- | --- |
| Lexer `~[` glue | drop the `TildeLBracket` push | `glued_tilde_bracket_parses_as_a_combinator_parameter` |
| `effect_has_variable`'s `TildeLBracket` arm | remove the match arm | `tilde_bearing_signature_always_routes_to_the_poly_parser` |
| `parse_slot`'s gate | remove the peek | `inline_quotation_as_extern_parameter_is_error` |
| `parse_type_expr`'s gate | remove the peek | `inline_quotation_as_array_element_is_error` (+ ref-referent golden) |
| `parse_field_type_expr`'s gate | remove the peek | `inline_quotation_as_struct_field_is_error` |
| Capture admission's `~` branch | remove the `InlineQuotation` check | `check_capture_admission_rejects_captured_inline_quotation` |
| `audit_poly_input_quotation`'s `Concrete`-arm recursion | collapse to `Concrete(_) \| Var(_) => Ok(())` | `inline_quotation_nested_in_a_quotation_parameter_is_error` |
| `apply_subst`'s `is_inline` branch | always ground to `quotation_type` | `variable_bearing_inline_quotation_still_mismatches_ordinary` |
| `raw_to_poly_type`'s fold `is_inline` branch | always fold to `Concrete(Type::Quotation)` | `inline_quotation_type_differs_from_ordinary_at_the_output_boundary` + both forwarding-mismatch goldens |

### Phase 3 — R4, R5, R6

| Guard | Mutation | Flips |
| --- | --- | --- |
| R6 fold suppression (`if !has_row`) | drop the guard, always fold when concrete | `parse_row_in_quotation_effect_survives_the_concrete_fold` |
| R4 `quotation_row_id` | always intern/accept any name | both `..._fresh_name_is_error` and `..._no_top_level_row_is_error` |
| R5 one-sided check | drop both `(Some, None)`/`(None, Some)` arms | `parse_row_in_quotation_effect_one_sided_is_error` |
| R5 differing-row check | drop the `a != b` arm | `parse_row_in_quotation_effect_differing_output_row_is_error` |

### Phase 4 — R9, R10

| Guard | Mutation | Flips |
| --- | --- | --- |
| R9 row prepend | `fresh` from `eff.inputs` only, ignoring `row` | `row_bearing_inline_quotation_grounds_and_runs`, `abstract_row_bearing_quotation_passes_down` (transitively, the mismatch and borrow tests too; standalone stays green, correctly) |
| R10 render strip | `actual` from full `result`, no `skip(row.len())` | `row_grounding_mismatch_strips_the_caller_region` (prints `[ i64 -- i64 i64 i64 ]`) |
| R9 type-only prepend | `row: &[Slot]`, prepend the caller's real slots | `grounded_row_region_is_type_only_so_a_caller_borrow_is_not_flagged` (false `borrows the enclosing place \`v\` (D3)`) |

### Phase 5 — R12 (gathered in this phase; phase 5 landed no standalone report)

The back-edge's explicit self-call argument check
(`src/check.rs:8754`-area, the `for (i, want) in ground_inputs.iter().enumerate()`
loop) is the only *new located error* phase 5 introduces (R11 is a
correctness rewrite of what the arm *produces*, not a diagnostic; R13/R14a
are "unchanged"/"extract-and-ignore" respectively — nothing to mutate).

Mutation: replaced the loop body with `let _ = i; let _ = want; break;` (skip
every check, `#[allow(unreachable_code)]` on the now-dead `found` binding),
against a scratch copy verified `diff`-identical to the tracked file
afterward:

```text
test check::tests::back_edge_rejects_mismatched_self_call_argument ... FAILED

thread '...' panicked at src/check.rs:15567:34:
called `Result::unwrap_err()` on an `Ok` value: ()
test result: FAILED. 0 passed; 1 failed; 0 ignored; 783 filtered out
```

Restored; `diff` against the pre-mutation copy: byte-identical. The witness
(`loopy`, `docs/phase4-slice10-spec.md`'s own note on why it's not a
placebo) diverges from the standalone check specifically so it cannot pass by
`'a` binding to `i64` universally: `main` instantiates `'a = str`, and only
the R12 unify — not the standalone check, which would accept the wrong `i64`
argument at `'a = i64` — catches the mismatched literal.

### Phase 6 — R14

Already recorded in full in `phase6_surviving_set_forwarding_gate.md`:
reverting `back_edge_outs`'s forward line to `let _ = src;` (leaving
`surviving: None`) flips `back_edge_outs_forwards_surviving_set_along_index_map`
(`left: None`, `right: Some(SurvivingCaptureSetId(0))`); restored,
`git diff --stat` touched only the intended lines.

## Green

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`:
full suite green, including the new `phase4_slice10a_exit_witnesses` (7
tests), `qbe_baseline` (byte-identical), and every prior phase's tests
unaffected. `git status --short` after the phase-5 and phase-1 mutation
reproductions above is clean (both restores verified byte-identical via
`diff` before running the suite).

10a exits here: `Type::InlineQuotation` and its audit (phase 1), `~[` surface
syntax and the five materialization boundaries (phase 2), rows in a
quotation effect (phase 3), row grounding at the four check-site contexts
(phase 4), the back-edge's ground declared outputs and bottom-aligned index
map (phase 5), the surviving-capture-set forward (phase 6), and the
user-space `my-times` witness with every guard shown capable of failing
(phase 7).
