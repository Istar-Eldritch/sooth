---

# Phase 4 Slice 6b: `filter`/`while`, and the self-tail combinator loop

**Status: implemented.** Base `main` @ `9600892`. Depends on 6a (inliner, `lib/combinators.sth`). Decisions D1–D9 locked (from `docs/phase4-slice6b-brief.md`).

`filter` ships as a library word with **no compiler change** (D1). The compiler deliverable is `while`, via a self-*tail* combinator loop (D3) — not via either roadmap "polymorphic-path gap." A combinator body is checked by term-splicing at the concrete call site, never through `poly_term`, so the polymorphic-`if` rejection (`src/check.rs:3672`) and the poly self-call hardcode (`src/ir.rs:1218`) never gate one and stay untouched. `while`'s only blocker was 6a's D5 combinator-cycle rejection, which fired even for a monomorphic self-recursive combinator. The work: relax that rejection for a self-*tail* edge, and lower that edge to a loop back-edge at splice time.

## Locked decisions

- **D1.** `filter` is a `Copy`-element combinator, no compiler change.
- **D2.** `filter`'s predicate is `[ 'T -- bool ]` (`dup`-predicate); element bound `'T: Copy` declared explicitly though inert.
- **D3.** The deliverable is `while`, via a self-tail combinator loop.
- **D4.** `while`: `( 'a [ 'a -- 'a bool ] -- 'a )`, body `| p | p call if p while else end`.
- **D5.** 6a's D5 relaxed *only* for a self-tail edge; non-tail self-call or mutual cycle stays `combinator_cycle_error`.
- **D6.** The polymorphic-body `if` gap and the polymorphic self-call gap left in place.
- **D7.** `filter`/`while` land in `lib/combinators.sth`.
- **D8.** The self-tail loop is a splice-time back-edge, not a specialized `IrFunc`. If the composition surfaces a phi-typing/staging mismatch neither ingredient hits alone, **reopen D8 rather than patch around it**.
- **D9.** `while` inherits the R18 nested-loop limit (not lifted here; slice 6d).

## Mechanism

Reuse, not invention. `begin_loop`/`finalize_loop` (`src/ir.rs:2393`, `:2440`) already open a mid-body loop at an arbitrary live stack; the `times` arm (`src/ir.rs:2597`) drives that with `stage_aggregates = true`, and the whole-word self-tail transform (`src/ir.rs:2066`) drives the same `begin_loop` with a self-call back-edge (`src/ir.rs:2928`). The self-tail combinator loop composes those two: mid-body opening from `times`, self-call back-edge from the whole-word transform.

**The three back-edge obligations transfer unchanged**, discharged at the self-call the way the whole-word transform does (`src/check.rs:6052`):
1. **Stack-row identity** — the `Copy` quotation `p` carries no `IrType`, is re-resolved statically each iteration, and is excluded from the loop-carried phis (as `times` pops its quotation before `begin_loop`). The runtime carried row is the threaded state `'a`. Enforced by ordinary stack-effect / `if`-join discipline; failure is the located `stack effect mismatch` / branch-join error. No new diagnostic.
2. **Move-state identity** — `check_linear_across_back_edge` (`src/check.rs:4851`); failure `linear_across_back_edge_error` (`:4789`), naming the live linear type and `while`.
3. **Borrow-state identity** — `check_reference_across_back_edge` (`src/check.rs:4830`); failure `reference_across_back_edge_error` (`:4812`), naming the borrowed place and `while`.

## Requirements by stage

### `filter` (`lib/combinators.sth`)
- **R1.** `filter` added, signature `( ['T: Copy 'N] [ 'T -- bool ] -- ['T 'N] usize )`, body the in-place `times` compaction; no compiler change.
- **R2** *(D2)*. Predicate `[ 'T -- bool ]`; body `dup`s the element, one copy to `p call`, keeps or drops the original. `'T: Copy` inert (no linear-element array can be built). Enforcement point is the concrete splice: a non-`Copy` element would fire the ordinary `cannot copy` on the body's `dup`; no definition-site poly check.
- **R3.** Write cursor threads on the stack below the `times` index (like `fold`'s accumulator); it is **not** rebound inside the body (a rebind type-checks but never advances).

### Relaxing D5 (`src/check.rs`)
- **R4** *(D5, located)*. `check_combinator_cycles` (`src/check.rs:5034`) relaxed: a self-edge is permitted iff every self-name occurrence is in tail position. Compare the self-name's `all_calls` count (`:3017`) against `tail_position_calls` (`:2673`) — equal-and-nonzero = tail-only (allow), unequal = non-tail (reject `combinator_cycle_error`, `:5091`). Any cycle of length ≥ 2 still rejected.
- **R5.** Recognizer reuses `has_self_tail_call`/`tail_position_calls`; both descend into `if` arms. A `drop` overload cannot be a self-tail combinator (`has_self_tail_call` special-cases it, `:2714`).

### Self-tail splice in the checker (`src/check.rs`)
- **R6.** `inline_combinator` (`src/check.rs:5117`) gains a self-tail branch: a current-combinator marker is set for the body splice; a tail-position call to that name is **not** re-spliced but treated as the loop back-edge, running the two obligation checks (`:6052`), then terminating that branch. Non-tail self-calls are already rejected by R4 before any splice.
- **R7** *(obligation 1)*. Stack-row identity by existing stack-effect/`if`-join discipline; no new diagnostic.
- **R8** *(obligation 2, located)*. `check_linear_across_back_edge` → `linear_across_back_edge_error`.
- **R9** *(obligation 3, located)*. `check_reference_across_back_edge` → `reference_across_back_edge_error`.

### Self-tail back-edge in lowering (`src/ir.rs`)
- **R10** *(D8)*. The combinator splice branch (`src/ir.rs:2868`) gains a self-tail branch: opens a mid-body loop with `begin_loop(&params, true)` carrying the runtime row only (quotation phantom excluded); saves/restores enclosing loop state (`header`/`entry_block`/`carried_slots`/`back_edges`) around the region so loops compose (as the `times` arm does). A tail self-call is emitted as a back-edge (`back_edges.push` + `Jmp(header)`, as `:2928`), not `Instr::Call`, not a re-splice. `finalize_loop` back-patches phis; a fall-through exit carries state out.
- **R11** *(carried aggregate)*. Reuses `stage_aggregates = true` verbatim (the slice-3 aggregate-return aliasing fix, known-fragile). Required test (criterion 8).
- **R12** *(empty `false` arm)*. `else end` falls through leaving `'a`; the monomorphic `if`-join already accepts this (the `countup` shape). Confirmation test (criterion 9).
- **R13** *(D4, D7)*. `while` added to `lib/combinators.sth`, signature and body per D4.

### Behaviour pins and guards
- **R14** *(D9, located)*. `while` brought **under** the R18 nested-loop limit; it does not inherit it automatically. Both directions closed explicitly:
  - **(a)** a `while` inside an open loop — opening a self-tail combinator loop while `loop_depth > 0` or an enclosing word is self-tail is the located rejection.
  - **(b)** a `times` inside a self-tail combinator body — the splice raises loop-open state so the `times` there is rejected.

  `times_depth` renamed to `loop_depth` (`src/check.rs:437`) since it now counts two kinds of loop; saved/restored across the splice (`:6009-6014`), not decremented, so sequential loops don't false-positive. The *limit* is not lifted here (slice 6d). Located-rejection tests: criteria 14, 14b.
- **R15** *(constant-stack witness)*. `while` and its hand-threaded monomorphic twin agree in exit code and stdout across a `ulimit -s` sweep at N=10k (`run_at_stack_limit`), plus a structural unit that the lowered `while` is a loop (header + back-edge, no self `Call`, no infinite splice). The 1M witness is unavailable (recon-2 fixed-array-codegen timeout, pre-existing).
- **R16** *(D6, out of scope)*. `src/check.rs:3672`, `poly_call_term`, and the `src/ir.rs:1218` hardcode untouched. Non-tail and mutual combinator cycles stay `combinator_cycle_error`. No runtime quotation value, calling convention, or `IrType` quotation variant enters this slice.
- **R17** *(REPL parity, located)*. `while`/`filter` at the REPL is 6c. The `eval_def` chokepoint (`src/repl.rs:1658` via `word_declares_quotation_parameter`, `src/check.rs:5006`) already rejects a self-tail combinator at definition; a test confirms the new path leaks no unpinned session case.
- **R18** *(dogfood)*. An `examples/` `while`/`filter` example paired with a hand-threaded twin (`examples/array_totals*.sth` pattern), golden-pinned to the same output. **Arrays passed inline / from a producer word**, never bound-then-passed (avoids 6a's bind-then-pass alias limitation, which is 6a's not a 6b regression). ROADMAP 6b marked implemented; DESIGN.md control-flow records the splice-time back-edge (D8) and the D5 tail-only relaxation.

## Invariants preserved

Backend stays QBE; `Ptr[T]` opaque; no LLVM, native backend, JIT, comptime. `IrType` gains no quotation variant. Linear spine untouched (quotation is `Copy`; R8/R9 preserve the spine at the new edge). `core` stays `no_std`. Constant stack preserved — `while` lowers to the same `begin_loop`/`finalize_loop` machinery. No new `Instr`/`Terminator`; no `qbe.rs` change. A program using no self-tail combinator lowers byte-for-byte as today. `stage_aggregates` reused verbatim, not modified.

## Delivery (as landed)

1. **(standard)** `filter` as a library word plus tests (R1–R3). `f0e39239`.
2. **(hard)** The whole `while` deliverable: relax D5 (R4–R5), the self-tail splice with its three obligations (R6–R9), the splice-time back-edge with the carried-aggregate and empty-arm paths (R10–R13), `while` in the library, and the behaviour pins (R14–R15). `28605062`. The checker relaxation and the lowering back-edge landed in **one** phase (relaxing without the back-edge would type-check then splice forever).
3. **(standard)** Out-of-scope guards, REPL parity, dogfood, docs (R16–R18). `47d02419`, `91db9f02` (review cycle 1).

## Exit criteria (goldens in `tests/phase4_combinators.rs`)

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | `filter` checks standalone at its signature | `filter_checks_standalone` | golden | 1 |
| 2 | `filter` over `[i64 8 3 9 1]` keeping `>4` inlines, runs, prints `2`, compacts in place | `filter_over_array_inlines_and_runs` | golden | 1 |
| 3 | `filter` over `[f64 …]` by a float predicate (element/length polymorphism) | `filter_is_element_polymorphic` | golden | 1 |
| 4 | a self-tail-only combinator edge is permitted | `self_tail_combinator_edge_is_allowed` | golden | 2 |
| 5 | a non-tail self-call is still `combinator_cycle_error`, naming the word | `non_tail_combinator_self_call_is_still_a_cycle_error` | golden | 2 |
| 6 | a two-combinator mutual cycle is still `combinator_cycle_error`, naming both | `mutual_combinator_cycle_is_still_an_error` | golden | 2 |
| 7 | `while` runs to fixpoint: `0 [ dup 5 < if 1 + true else false end ] while .` prints `5` | `while_runs_to_a_fixpoint` | golden | 2 |
| 8 | a `while` threading an aggregate state runs (the `stage_aggregates` path) | `while_carrying_an_aggregate_state_runs` | golden | 2 |
| 9 | `while`'s empty `else end` arm falls through leaving the state | `while_empty_false_arm_falls_through` | golden | 2 |
| 10 | outer linear local live across the back-edge is `linear_across_back_edge_error` | `while_body_linear_local_across_back_edge_is_error` | golden | 2 |
| 11 | reference to a frame local across the back-edge is `reference_across_back_edge_error` | `while_body_reference_across_back_edge_is_error` | golden | 2 |
| U12 | the lowered `while` is a loop (header + back-edge, no self `Instr::Call`, no re-splice) | `while_lowers_to_a_back_edge_not_an_infinite_splice` | unit | 2 |
| 13 | `while` and its hand-threaded twin agree across a `ulimit -s` sweep at N=10k | `while_and_hand_threaded_loop_agree_across_stack_limits` | golden | 2 |
| 14 | a `while` inside a `times` body is the located R18 rejection (R14a) | `while_nested_in_a_loop_is_rejected` | golden | 2 |
| 14b | a `times` inside a self-tail combinator body is the located R18 rejection (R14b) | `times_inside_a_self_tail_combinator_is_rejected` | golden | 2 |
| 15 | a session line defining a self-tail combinator is still the located REPL rejection (6c) | `repl_self_tail_combinator_definition_is_rejected` | golden | 3 |
| 16 | dogfood over `filter`/`while` (arrays inline) matches its hand-threaded twin | `filter_while_dogfood_matches_hand_threaded` | golden | 3 |

**14/14b are load-bearing, not paperwork** — each must fail before its half of R14 lands (without R14a the nested `while` miscompiles; without R14b the nested `times` does). Mutation-test the guards: **U12** carries the constant-stack guarantee (deleting the back-edge branch must fail with an `Instr::Call` to `while` or an infinite splice); **10/11** must fail if their `check_*_across_back_edge` call is removed; **5/6** must fail (accept a bad program) if the tail-only condition in R4 is deleted.

## Sanctioned edits

`lib/combinators.sth` (+ `export:` line) gains `filter` and `while`. `src/check.rs` (R4–R9, R14 rename) and `src/ir.rs` (R10–R12). `ROADMAP.md` 6b marked implemented; `DESIGN.md` control-flow records D5's relaxation and the D8 splice-time back-edge. `examples/filter_while.sth`, `examples/filter_while_hand.sth`, and tests. No `src/check.rs:3672` / `src/ir.rs:1218` rejection lifted; no behaviour a non-combinator program relies on changes.
