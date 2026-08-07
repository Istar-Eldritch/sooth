# Phase 4 Slice 6b: `filter`/`while`, and the self-tail combinator loop

**Status: specified.** Base `main` @ `9600892`. Depends on 6a for the inliner and the
library's file/shape (`lib/combinators.sth`). The authoritative discovery document is
[`docs/phase4-slice6b-brief.md`](./phase4-slice6b-brief.md); its recon items were built and
run against the compiler, and its decisions **D1–D9 are locked** and specified here, not
re-litigated.

`filter` needs **no compiler change** and ships as a library word (D1). The entire compiler
deliverable is `while`, and its blocker is neither of the two "polymorphic-path gaps" the
roadmap named: a combinator body is checked by term-splicing at the *concrete* call site,
never through `poly_term`, so the polymorphic-`if` rejection (`src/check.rs:3672`) never
gates one, and a polymorphic body cannot call any polymorphic word at all (the
`src/ir.rs:1218` poly-instantiation `self_tail` hardcode is unreachable). `while`'s only
blocker is 6a's own D5 combinator-cycle rejection, which fires identically for a
*monomorphic* self-recursive combinator (verified: the body
`: while ( 'a [ 'a -- 'a bool ] -- 'a ) | p | p call if p while else end ;` → `` a
quotation-taking word cannot be recursive … `while` -> `while` ``). The work is to relax
that rejection for a self-*tail* combinator edge only,
and lower that edge to a loop back-edge at splice time.

## Fixed shape (verified against the built tree)

1. **`filter` already compiles and runs, unchanged.** Built and run: `mk [ 4 > ] filter`
   over `[i64 8 3 9 1]` prints `2` (kept `8` and `9`), exit `0`. It compacts in place
   through `&arr i &> @` reads and `&!arr over &!> v !` writes, threading a write cursor on
   the stack, so it needs no fresh allocation and no dynamic collection. This is 6a's
   inliner and nothing else.
2. **`while`'s blocker is a checker rejection, not an IR gap.** The loop shape it needs
   already runs: a monomorphic non-combinator self-tail word with its condition inlined
   (`: countup ( i64 -- i64 ) dup 1000000 < if 1 + countup else end ;`) runs in constant
   stack. The missing piece is purely: route a self-*tail* call *inside a combinator* to a
   loop back-edge instead of splicing forever. The quotation is statically the same literal
   on every iteration (`Copy`, 6a D3), so **the loop needs no runtime quotation value** —
   this is a loop, not slice-7 territory.
3. **The mechanism is reuse, not invention (D8).** `begin_loop`/`finalize_loop`
   (`src/ir.rs:2393`, `:2440`) already open a mid-body loop at an arbitrary live stack: the
   `times` arm (`src/ir.rs:2597`) drives exactly that path with `stage_aggregates = true`,
   and the whole-word self-tail transform (`src/ir.rs:2066`) drives the same `begin_loop`
   with a self-call-driven back-edge (`src/ir.rs:2928`). The self-tail combinator loop is
   the *composition* of those two existing ingredients — mid-body opening from `times`, a
   self-call back-edge from the whole-word transform — and neither ingredient is new.

## Locked decisions (from the brief; specified, not reopened)

- **D1.** `filter` ships as a `Copy`-element combinator with no compiler change.
- **D2.** `filter`'s predicate is `[ 'T -- bool ]` (`dup`-predicate), element bound
  `'T: Copy`, declared explicitly though currently inert.
- **D3.** The real deliverable is `while`, via a self-*tail* combinator loop, not via either
  roadmap gap.
- **D4.** `while`'s signature is `( 'a [ 'a -- 'a bool ] -- 'a )`, body
  `| p | p call if p while else end`.
- **D5.** 6a's D5 is relaxed *only* for a self-tail combinator edge; a non-tail self-call or
  a mutual cycle stays `combinator_cycle_error`.
- **D6.** The polymorphic-body `if` gap and the polymorphic self-call gap are left in place,
  untouched.
- **D7.** `filter` and `while` land in the existing `lib/combinators.sth`.
- **D8.** The self-tail loop is a splice-time back-edge, not a specialized `IrFunc`.
  Specialization was weighed and rejected (it reopens 6a's "inlining is total" and "a
  combinator mints no symbol").
- **D9.** `while` inherits the R18 nested-loop limit, accepted, not fixed here (slice 6d).

## The load-bearing claim: the three back-edge obligations transfer unchanged

This is the brief's lead open question and what a reviewer should attack first. A loop
back-edge must discharge three obligations. The whole-word self-tail transform enforces all
three by walking a body the checker can see, at the self-call site (`src/check.rs:6052`):

```rust
if tail && ctx.mangled_name() == Some(name.as_str()) {
    check_linear_across_back_edge(ctx, span, name, &stack[..base], scope, arrays)?;
    check_reference_across_back_edge(ctx, span, name, &stack[base..], prov)?;
}
```

The claim is that a self-tail *combinator* call reaches the same three checks with the same
outcome, because the carried row here (the caller's live stack) is not novel: the whole-word
transform already carries a caller-derived param row through `begin_loop(&params, true)`
(`src/ir.rs:2066`), and `times` already carries the caller's live stack plus an index
through the same call (`src/ir.rs:2597`). Taken in turn:

1. **Stack-row identity (back-edge row = header row).** The loop-carried *runtime* row is
   the threaded state `'a` (the `Copy` quotation `p` carries no `IrType`, is the same literal
   every iteration, and is re-resolved statically at each splice, so it is **excluded from
   the loop-carried phis**, exactly as `times` pops its quotation before `begin_loop`). The
   header row is `while`'s declared input state; the back-edge row is what `p while` presents
   in the `true` arm. Identity is enforced by the checker's ordinary stack-effect discipline:
   the self-call must present `while`'s declared input row (`( 'a [effect] )`) and the base
   (`else end`) arm must leave `'a`, so the `if`-join forces both arms — and therefore header
   and back-edge — to the same row. **Diagnostic on failure:** the ordinary
   `stack effect mismatch` / branch-join mismatch already raised by the monomorphic `if`
   machinery, located at the offending term. No new diagnostic.
2. **Move-state identity (no outer linear local consumed, or disposed once per iteration).**
   `check_linear_across_back_edge` (`src/check.rs:4851`) over `stack[..base]` (the caller's
   residual, carried unchanged). For `while`, `p` is `Copy` (6a D3) and the state `'a` is
   threaded (consumed then re-produced), not a captured outer linear local, so nothing linear
   is live across the edge and the check passes. **Diagnostic on failure:**
   `linear_across_back_edge_error` (`src/check.rs:4789`), located at the self-tail call, its
   message naming the live `'a` type and the callee `while`.
3. **Borrow-state identity (no reference crossing the back-edge).**
   `check_reference_across_back_edge` (`src/check.rs:4830`) over `stack[base..]` (the
   self-call inputs). A reference to a frame local threaded as the state would cross the edge
   and be rejected. **Diagnostic on failure:** `reference_across_back_edge_error`
   (`src/check.rs:4812`), located at the self-tail call, naming the borrowed place and
   `while`.

**Where the claim could break, and the honest reopening condition.** The one genuinely new
thing is the *composition*: a mid-body-opened loop (from `times`) whose back-edge is driven
by a self-call (from the whole-word transform), rather than by a synthesized index. Both
ingredients feed the identical `begin_loop`/`finalize_loop` pair, and a back-edge from inside
an `if` arm is already lowered and tested
(`both_if_arms_tail_produce_two_back_edges`, `src/ir.rs:6314`). If, and only if, the
composition surfaces a phi-typing or staging mismatch that neither ingredient hits alone (the
carried row being the caller's stack rather than a synthesized index), the specialization
option rejected in D8 becomes attractive again, and **D8 should be reopened rather than
patched around**. The spec asserts the obligations transfer; it does not license a stopgap if
they do not.

## Requirements by stage

Located diagnostics assert message text **and** named identifiers/positions.

### `filter` as a library word (`lib/combinators.sth`)

- **R1.** `filter` is added to `lib/combinators.sth` and its `export:` line, signature
  `( ['T: Copy 'N] [ 'T -- bool ] -- ['T 'N] usize )`, body the in-place compaction over
  `times` (the recon-2 program). **No compiler change.** It type-checks and lowers entirely
  through 6a's already-shipped inliner.
- **R2** *(D2)*. The predicate is `[ 'T -- bool ]`; the body `dup`s the element, hands one
  copy to `p call`, keeps or drops the original. The element bound `'T: Copy` is declared
  explicitly. It is currently inert — no array of a non-`Copy` element can be built (`fill`
  rejects a linear element, `src/check.rs`), so no such type reaches a `filter` splice — but
  it is the honest constraint (`dup` requires it). **The enforcement point is the splice
  site:** a combinator body is never checked through `poly_term`, so if a non-`Copy`-element
  array ever reached `filter`, the ordinary `cannot copy` on the body's `dup` at the concrete
  splice would fire. That is the accepted enforcement point; there is no definition-site poly
  check carrying the bound.
- **R3** *(recon 9)*. The write cursor threads on the stack, below the `times` index, exactly
  as `fold` threads its accumulator (`0 n [ | i | … ] times`, the body's net effect
  `( w i -- w' )`). It is **not** rebound inside the body (`w 1 + | w2 |`), which type-checks
  but silently never advances, because `times` forbids carrying move-state across the
  back-edge and a rebind is a fresh per-iteration local.

### Relaxing D5 for a self-tail edge (`src/check.rs`)

- **R4** *(D5, located)*. `check_combinator_cycles` (`src/check.rs:5034`) is relaxed: a
  **self-edge is permitted iff every occurrence of the self-name in the body is in tail
  position**. A self-name occurring in *any* non-tail position, and any cycle of length ≥ 2,
  still returns `combinator_cycle_error` (`src/check.rs:5091`) unchanged. Today the edge set
  is built from `all_calls` (`src/check.rs:3017`), which erases the tail/non-tail
  distinction; the relaxation compares the self-name's `all_calls` count against its
  `tail_position_calls` count (`src/check.rs:2673`) — equal-and-nonzero means tail-only
  (allow, hand to the loop transform), unequal means a non-tail occurrence (reject).
- **R5.** The tail-vs-non-tail recognizer reuses `has_self_tail_call`/`tail_position_calls`;
  no new AST walker. Both already descend into `if` arms and match by name, so they
  structurally recognize `while`'s recursion (`p while` is the last term of the `if`'s
  `true` arm). A monomorphic `drop` overload cannot be a self-tail combinator (`drop` is
  never a combinator and `has_self_tail_call` already special-cases it, `src/check.rs:2714`).

### The self-tail splice in the checker (`src/check.rs`)

- **R6.** `inline_combinator` (`src/check.rs:5117`) gains a self-tail branch. When the
  combinator being inlined is self-tail (R5), a **current-combinator marker** is set for the
  duration of the body splice. A tail-position call to that same combinator name reached
  inside the spliced body is **not re-spliced** (which would recurse forever); it is treated
  as the loop back-edge and runs the same two obligation checks the whole-word self-tail path
  runs at `src/check.rs:6052`, then terminates that branch. A non-tail self-call inside the
  body is already rejected by R4 at `check_combinator_cycles`, before any splice, so it
  cannot reach this branch.
- **R7** *(obligation 1)*. Stack-row identity is discharged by the existing stack-effect and
  `if`-join discipline (see §load-bearing claim). No new diagnostic; a mismatch is the
  ordinary located `stack effect mismatch` / branch-join error.
- **R8** *(obligation 2, located)*. Move-state identity via `check_linear_across_back_edge`
  (`src/check.rs:4851`) at the self-tail call; failure is `linear_across_back_edge_error`
  (`src/check.rs:4789`), naming the live linear type and `while`.
- **R9** *(obligation 3, located)*. Borrow-state identity via
  `check_reference_across_back_edge` (`src/check.rs:4830`) at the self-tail call; failure is
  `reference_across_back_edge_error` (`src/check.rs:4812`), naming the borrowed place and
  `while`.

### The self-tail back-edge in lowering (`src/ir.rs`)

- **R10** *(D8)*. The combinator splice branch (`src/ir.rs:2868`) gains a self-tail branch.
  For a self-tail combinator it opens a mid-body loop with `begin_loop(&params, true)`
  (`src/ir.rs:2393`), carrying the **runtime** row only (the threaded state; the `Copy`
  quotation phantom is excluded — it has no `IrType` and is re-resolved statically each
  iteration, exactly as the `times` arm pops its quotation before `begin_loop`). It saves and
  restores the enclosing loop state (`header`/`entry_block`/`carried_slots`/`back_edges`)
  around the region so loops compose, exactly as the `times` arm does (`src/ir.rs:2597`).
  A tail-position call to the same combinator inside the spliced body is emitted as a
  back-edge (`back_edges.push` + `Jmp(header)`) exactly as the whole-word self-tail branch at
  `src/ir.rs:2928`, not as an `Instr::Call` and not as a re-splice. `finalize_loop`
  (`src/ir.rs:2440`) back-patches the phis; a fall-through exit block carries the state out.
- **R11** *(carried aggregate)*. The state `'a` may be an aggregate. The self-tail back-edge
  reuses `stage_aggregates = true` verbatim: `begin_loop` hoists one stable slot per carried
  aggregate into the entry block and `finalize_loop` appends the unconditional
  read-before-write staged blit on the back-edge (the Phase 4 slice-3 aggregate-return
  aliasing fix, the known-fragile invariant in this code). A carried-aggregate `while` is a
  required test (criterion 8).
- **R12** *(empty `false` arm)*. `while`'s `else end` arm is empty; it must fall through
  leaving the state `'a` on the stack. The monomorphic `if`-join already accepts an empty arm
  whose only content is the incoming stack (the `countup` witness has exactly this shape and
  builds), so this is a confirmation pinned as a test (criterion 9), not new machinery.
- **R13** *(D4, D7)*. `while` is added to `lib/combinators.sth` and its `export:` line,
  signature `( 'a [ 'a -- 'a bool ] -- 'a )`, body `| p | p call if p while else end`.

### Behaviour pins and out-of-scope guards

- **R14** *(D9, located)*. `while` must be brought **under** the R18 nested-loop limit. It
  does **not** inherit it automatically, and specifying otherwise would ship a silent
  miscompile. The existing guard fires only at a `times` term (`src/check.rs:5889`):

  ```rust
  let in_self_tail = matches!(ctx, Ctx::Word { self_tail: true, .. });
  if in_self_tail || prov.times_depth > 0 {
      return Err(times_nested_in_loop_error(ctx, span));
  }
  ```

  Neither trigger covers a spliced self-tail combinator loop. `in_self_tail` reads the
  *enclosing word's* `Ctx`, which for a spliced `while` is the caller (`main`), not `while`;
  and `times_depth` counts only open `times` body splices. So **both** directions are
  unguarded today and each must be closed explicitly:
  - **(a) a `while` sited inside an open loop.** `… [ … c::while ] times` reaches the
    combinator splice, not a `times` term, so nothing fires and lowering would open a second
    `begin_loop` with the wrong hoist target. Opening a self-tail combinator loop while a
    loop is open (`times_depth > 0` or `in_self_tail`) must be the located rejection.
  - **(b) a `times` inside a self-tail combinator body.** While splicing `while`'s body the
    loop-open state must be *raised*, or a `times` there is accepted and nests. The
    self-tail splice must increment and restore the same counter the `times` arm does
    (`src/check.rs:5914-5919`, saved and restored rather than decremented, so sequential
    loops do not false-positive).

  Since the counter now counts two kinds of loop, rename `times_depth` to `loop_depth` and
  update its doc comment (`src/check.rs:418-425`), rather than leaving a name that lies.
  The *limit* is pre-existing and **not lifted here** (slice 6d lifts it for all five
  combinators at once); what this slice owes is that `while` is subject to it rather than
  slipping past it. Both directions are pinned as located-rejection tests (criteria 14, 14b).
- **R15** *(constant-stack witness)*. A `while` and its hand-threaded monomorphic loop twin
  agree in exit code and stdout across a `ulimit -s` sweep at N = 10k, reusing
  `run_at_stack_limit` (the `filter`/`countup` precedent), plus a structural unit that the
  lowered `while` is a loop (header + back-edge, no self `Call`, no infinite splice). The 1M
  witness is unavailable (recon 2's fixed-array-codegen timeout, pre-existing); the spec does
  not specify a run that cannot build.
- **R16** *(D6, out of scope)*. The polymorphic-body `if` (`src/check.rs:3672`) and
  polymorphic self-call resolution (`poly_call_term`) are left exactly as they are; the
  `src/ir.rs:1218` poly-instantiation `self_tail` hardcode is unreachable and D8 routes
  around it, so it too is untouched. Non-tail combinator recursion and mutual combinator
  cycles stay `combinator_cycle_error` (they need slice 7's runtime quotation values). No
  runtime quotation value, calling convention, or `IrType` quotation variant enters this
  slice.
- **R17** *(REPL parity guard, located)*. `while`/`filter` at the REPL is slice 6c. The
  existing chokepoint (`eval_def`, `src/repl.rs:1658`, via `word_declares_quotation_parameter`,
  `src/check.rs:5006`) keys on the declared quotation parameter, so it already rejects a
  self-tail combinator at definition. A test confirms the new self-tail path leaks **no**
  unpinned session case (the slice-1 lesson): a session line defining `while` is still the
  located `declares a quotation parameter … slice 6c` rejection.
- **R18** *(dogfood, recon 10)*. An `examples/` `while`/`filter` example paired with a
  hand-threaded twin (the `examples/array_totals*.sth` pattern), the golden pinning both to
  the same output. **Arrays are passed inline / from a producer word**, never bound to a
  local and then passed, so the 6a bind-then-pass alias limitation (recon 10:
  `cannot borrow … it is aliased by …`) is not tripped. That limitation is 6a's, not a 6b
  regression; the spec states so plainly. ROADMAP slice 6b is marked implemented; DESIGN.md's
  control-flow section records the self-tail combinator loop as a splice-time back-edge (D8)
  and the relaxation of D5 for a tail-only self-edge (D5).

## Invariants preserved

Backend stays QBE; `Ptr[T]` opaque; no LLVM, native backend, JIT, comptime. `IrType` gains
no quotation variant. Linear spine untouched: a quotation parameter is `Copy` (6a D3), and
the linear/reference-across-back-edge checks (R8/R9) preserve the spine at the new loop edge.
`core` stays `no_std`. Constant stack preserved: a self-tail combinator lowers to the same
`begin_loop`/`finalize_loop` back-edge machinery `times` and the whole-word transform use, so
`while` runs in constant stack. No new `Instr`/`Terminator`; no `qbe.rs` change. A program
using no self-tail combinator lowers byte-for-byte as today. The slice-3 aggregate-return
aliasing fix (`stage_aggregates`) is reused verbatim, not modified.

## Delivery

Three phases, each ending green (`cargo fmt --check && cargo clippy --all-targets -- -D
warnings && cargo test`). The checker relaxation (R4–R9) and the lowering back-edge (R10–R13)
must land in **one** phase: relaxing `check_combinator_cycles` without the lowering back-edge
would let `while` type-check and then splice forever at lowering, which is not a green state.

1. **(standard)** `filter` as a library word plus tests (R1–R3). Pure library, no compiler
   change; green immediately. This is the obvious early phase D1 makes possible.
2. **(hard)** The whole `while` deliverable: relax D5 for a tail-only self-edge (R4–R5), the
   self-tail splice in the checker with its three obligations (R6–R9), the splice-time
   back-edge in lowering with the carried-aggregate path and empty-arm confirmation
   (R10–R13), `while` in the library, and the behaviour pins (R14–R15). The load-bearing,
   escalated phase: it is where the D8 obligation claim is proved or the decision reopened.
3. **(standard)** Out-of-scope guards, REPL parity, dogfood, and docs (R16–R18). No
   behaviour a non-combinator program relies on changes.

## Exit criteria (goldens in `tests/phase4_combinators.rs`)

Located diagnostics assert message text and named identifiers. Each row names the test
function the implementation must add.

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | `filter` checks standalone at its declared signature | `filter_checks_standalone` | golden | 1 |
| 2 | `filter` over `[i64 8 3 9 1]` keeping `>4` inlines, runs, prints `2` and compacts in place | `filter_over_array_inlines_and_runs` | golden | 1 |
| 3 | `filter` over an `[f64 …]` array keeps by a float predicate (element/length polymorphism) | `filter_is_element_polymorphic` | golden | 1 |
| 4 | a self-tail-only combinator edge is permitted (`while` checks standalone; no cycle error) | `self_tail_combinator_edge_is_allowed` | golden | 2 |
| 5 | a combinator with a **non-tail** self-call is still `combinator_cycle_error`, naming the word | `non_tail_combinator_self_call_is_still_a_cycle_error` | golden | 2 |
| 6 | a two-combinator mutual cycle is still `combinator_cycle_error`, naming both members | `mutual_combinator_cycle_is_still_an_error` | golden | 2 |
| 7 | `while` runs to a fixpoint: `0 [ dup 5 < if 1 + true else false end ] while .` prints `5` | `while_runs_to_a_fixpoint` | golden | 2 |
| 8 | a `while` threading an **aggregate** state runs correctly (the `stage_aggregates` path) | `while_carrying_an_aggregate_state_runs` | golden | 2 |
| 9 | `while`'s empty `else end` arm falls through leaving the state | `while_empty_false_arm_falls_through` | golden | 2 |
| 10 | a `while` body keeping an outer **linear** local live across the back-edge is `linear_across_back_edge_error` | `while_body_linear_local_across_back_edge_is_error` | golden | 2 |
| 11 | a `while` body carrying a **reference** to a frame local across the back-edge is `reference_across_back_edge_error` | `while_body_reference_across_back_edge_is_error` | golden | 2 |
| U12 | the lowered `while` is a loop (header + back-edge, no self `Instr::Call`, no re-splice) | `while_lowers_to_a_back_edge_not_an_infinite_splice` | unit | 2 |
| 13 | `while` and its hand-threaded loop twin agree in exit code and stdout across a `ulimit -s` sweep at N=10k, and the `while` sums right at a generous limit | `while_and_hand_threaded_loop_agree_across_stack_limits` | golden | 2 |
| 14 | a `while` called inside a `times` body is the located R18 nested-loop rejection (D9, R14a) | `while_nested_in_a_loop_is_rejected` | golden | 2 |
| 14b | a `times` inside a self-tail combinator body is the located R18 nested-loop rejection (D9, R14b) | `times_inside_a_self_tail_combinator_is_rejected` | golden | 2 |
| 15 | a session line defining a self-tail combinator is still the located REPL rejection (6c) | `repl_self_tail_combinator_definition_is_rejected` | golden | 3 |
| 16 | dogfood over `filter`/`while` (arrays passed inline) matches its hand-threaded twin | `filter_while_dogfood_matches_hand_threaded` | golden | 3 |

**14/14b are load-bearing, not paperwork.** Each must be shown to fail before its half of R14
lands: without R14a the nested `while` compiles (and miscompiles) instead of being rejected,
and without R14b the nested `times` does. A test that passes before the guard exists is a
placebo, and this repo has shipped several.

Load-bearing units (mutation-test the guards, per project convention): **U12** carries the
constant-stack guarantee — deleting the back-edge branch must make it fail with an
`Instr::Call` to `while` or an infinite splice, not silently pass. **10/11** are the
move/borrow obligation pair (R8/R9): each must fail if its `check_*_across_back_edge` call is
removed from the self-tail splice path. **5/6** guard that the D5 relaxation (R4) did not
widen to accept a non-tail or mutual cycle; deleting the tail-only condition must make one of
them accept a program that should be rejected.

## Sanctioned edits

`lib/combinators.sth` and its `export:` line gain `filter` and `while`. `ROADMAP.md`'s slice
6b entry is marked implemented and `DESIGN.md`'s control-flow section records D5's relaxation
and the D8 splice-time back-edge. No behaviour a non-combinator program relies on changes; no
`src/check.rs:3672`/`src/ir.rs:1218` rejection is lifted.

```json
{
  "phases": [
    { "phase": 1, "focus": "filter as a library word in lib/combinators.sth plus its standalone-check, in-place-compaction, and element-polymorphism tests; no compiler change (R1-R3).", "difficulty": "standard" },
    { "phase": 2, "focus": "The while deliverable: relax check_combinator_cycles for a tail-only self-edge (R4-R5); the self-tail splice in the checker discharging the three back-edge obligations (R6-R9); the splice-time loop back-edge in lowering reusing begin_loop/finalize_loop with the carried-aggregate stage_aggregates path and the empty-false-arm confirmation (R10-R13); while in the library; and the nested-loop-limit and constant-stack behaviour pins (R14-R15).", "difficulty": "hard" },
    { "phase": 3, "focus": "Out-of-scope guards left in place, REPL definition-chokepoint parity for a self-tail combinator, the filter/while dogfood matched against a hand-threaded twin with arrays passed inline, and ROADMAP/DESIGN docs (R16-R18).", "difficulty": "standard" }
  ]
}
```
