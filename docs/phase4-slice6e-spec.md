# Phase 4 Slice 6e: `if` in a polymorphic body

Lifts the unconditional rejection of `if` inside a non-combinator polymorphic body so a poly word may branch. **Checker-acceptance only**: no new `Instr`/`Terminator`/lowering. Concrete instantiation already works once the poly-walk stops rejecting; lowering happens per instantiation via the existing `check_word`.

## Where it landed

- Dispatch by `is_combinator` (`check.rs:1385`): combinators → `check_poly_combinator_standalone`; other poly words → `check_poly_body` → `poly_walk` → `poly_term`.
- The rejection was a single unconditional early-out in `poly_term`'s `TermKind::If` arm — the clean insertion point, now replaced with the full arm.
- Ported from the monomorphic `if` arm in `check_term` (pop cond, `Bool`-check, clone scope/stack per branch, `leave_block` each, compare residual depth/types, `Moves::join`), minus all quotation handling.
- Reuses `MoveState`/`Moves` (`take`, `moved_site`, `unconsumed`, `join`). `PolyType: Eq` (`ast.rs:513`) gives direct per-slot `==` at the join.

## Locked decisions

- **D1** No new IR, checker only. Edits confined to `src/check.rs`, `tests/phase4_generics.rs`, `examples/poly_if.sth`, `ROADMAP.md`. `PolyType`/`PolySig`/`apply_subst`/monomorphization untouched.
- **D2** Three move states (live / moved-both-arms / moved-one-arm), realized by reusing `Moves` (Q1).
- **D3** No quotation machinery on the poly side: a `PolyType` is provably never a quotation (rejected eagerly at `poly_term`'s `Quotation` arm). So the poly `if` does **not** port `reject_quotation_operand`, `different_quotations_at_join_error`, or `quotation_versus_value_at_join_error` — dead by construction.
- **D4** Nested `if` falls out of recursion through `poly_walk`; no special case. One dogfood nests to check it.
- **D5** The rejection test was rewritten (not duplicated) into the primary acceptance test.

## Resolved questions

- **Q1 — PolyScope move tracking.** `moves: HashMap<..>` → `moves: Moves`. Binder inserts `MoveState::Live`; local read uses `moves.take(name, span)`; `unconsumed()` delegates; the body's final leak check now counts `MaybeMoved` (one-arm consumption) as leaked. Added `snapshot()` (keys set before an arm) and `leave_arm(&before, token)`: reject the first arm-local still unconsumed (name-sorted), then remove **every** arm-local from `locals` and `moves.states` so both arms' name sets agree and `join` cannot panic. Keys-snapshot, not an ordered `Vec` — `PolyScope` has no depth concept and reports leaks name-sorted; an arm-local can never be `MaybeMoved`.
- **Q2 — Three new `poly_`-prefixed `ctx`/`span` error fns** (beside `poly_use_after_move_error`), not the `word`/`sig` family, not generalized `branch_*_error` (those take `Type`, poly compares `PolyType`): `poly_branch_mismatch_error`, `poly_branch_type_mismatch_error`, `poly_arm_local_unconsumed_error`. Each has `Ctx::Word` and `Ctx::Line` phrasings mirroring monomorphic message text.
- **Q3 — Second concrete type is `f64`** (satisfies `Copy Ord` for `mymax`; `is_ord` is numeric-only so no struct qualifies). Matrix `{i64, f64}` for both words.
- **Q4 — `check_poly_combinator_standalone` unchanged** (confirmed by reading): combinators build an `i64` stand-in `WordDef` and run the monomorphic `check_word`, never touching `poly_term`. 6e is confined to the non-combinator poly path.

## The `poly_term` `if` arm

Destructures `TermKind::If { then_branch, else_branch, else_span, end_span }`:
1. Pop condition; underflow → `underflow_error`.
2. Require `PolyType::Concrete(Type::Bool)`; else `type_mismatch_error`. No quotation guard (D3).
3. `before = scope.snapshot()`; clone scope into then/else and stack into each arm.
4. Walk then-branch → `leave_arm(&before, then_token)` (`"else"` if `else_span.is_some()` else `"end"`).
5. Walk else-branch → `leave_arm(&before, "end")`.
6. `scope.moves = Moves::join(then, else)`.
7. Depth mismatch → `poly_branch_mismatch_error`.
8. Per-slot `t_then != t_else` → `poly_branch_type_mismatch_error`. No quotation-identity or deriv/place merge (monomorphic-only).
9. Return the merged residual stack.

## Exit criteria (golden tests)

| ID | Test | Kind | Assertion |
|----|------|------|-----------|
| T1 | `check_poly_body_with_if_accepts_choose` (rewrite of `..._is_rejected`) | unit accept | `choose ( 'T 'T bool -- 'T )` compiles |
| T2 | `check_poly_arm_local_unconsumed_is_error` | unit reject | arm-local `y` unconsumed → err naming `y`, "never consumed" |
| T3 | `check_poly_if_moved_on_both_arms_is_accepted` | unit accept | moved on both arms → ok |
| T4 | `check_poly_if_moved_on_one_arm_leaks` | unit reject | moved one arm → leak on `x` |
| T5 | `check_poly_if_moved_on_neither_arm_leaks` | unit reject | untouched on both → leak on `x` |
| T6 | `check_poly_if_condition_not_bool_is_error` | unit reject | non-bool cond → type mismatch |
| T7 | `check_poly_if_branch_depth_mismatch_is_error` | unit reject | `'T: Copy` depth mismatch (Copy needed or use-after-move masks it) |
| T8 | `check_poly_if_use_after_join_is_error` | unit reject | both arms consume `x`, later `x drop` → use-after-move |
| T9 | `poly_mymax_runs_at_i64_and_f64` (tests) | e2e | `mymax` at i64 (`3 7`→`7`) and f64 (`3.0 7.0`→`7`) |
| T10 | `poly_choose_runs_at_i64_and_f64` (tests) | e2e | `choose` at both, prints kept operand |
| T11 | `poly_nested_if_dogfood_runs` (tests) | e2e | build+run `examples/poly_if.sth` (nested `mymax3`) |
| T12 | ROADMAP 6e marked implemented; slice-7 dep note | doc | prose only |

### Mutation-test-required guards

- **M1 (T1)**: mutate `Moves::join` `(Moved,Moved)=>MaybeMoved` — T1 must fail (`choose`'s different-site consumption wrongly read as leak).
- **M2 (T2)**: delete leak-detection branch in `leave_arm` (keep removal) — T2 must fail-to-fail.
- **M3 (T3/T4/T5)**: mutate disagreement case → `Moved`, and `unconsumed` to exclude `MaybeMoved` — T4 must fail-to-fail both times; T3 must keep passing.
- **M4 (T5)**: `(Live,Live)=>Moved` — T5 must fail-to-fail (M3's mutations can't touch a `Live`+`Live` local).

## Delivery (as implemented)

- **Phase 1** (`3d61a7e`, hard) — whole `src/check.rs` change: `Moves` reuse in `PolyScope`, `snapshot`/`leave_arm`, three error fns, the `if` arm, T1 rewrite + T2–T8. Machinery landed with the `if` arm (no standalone private-function tests — placebo-prone); every test drives real source through `check_src`. M1–M3 verified.
- **Phase 2** (`2b847cd`) — `tests/phase4_generics.rs` T9/T10/T11 via `run_src`; new `examples/poly_if.sth` (`mymax`, `choose`, nested `mymax3`, `main` at both instantiations).
- **Phase 3** (`835bd87`, +review cycles `2fad343`, `b0d77c2`) — `ROADMAP.md`: 6e marked implemented, slice-7 dependency confirmed satisfied.

## Out of scope

- Moving `max`/`mymax` out of `BUILTIN_WORDS`.
- The quotation-in-poly-body rejection (`check.rs:3692`) — slice 7's wall.
- Any change to `PolyType`/`PolySig`/`apply_subst`/monomorphization.
- `check_poly_combinator_standalone` — unaffected (Q4).
- `while`/`times` inside a poly `if` arm — dispatched unchanged through recursive `poly_walk`; call out if implementation surfaces an interaction.
