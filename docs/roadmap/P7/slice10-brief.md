# Phase 7 Slice 10: bound the splice, diagnose the recursive impl (brief)

## Trigger

P7.S8 made `lib/cmp.sth`'s six surface comparisons (`eq`/`lt`/`gt`/`lte`/`gte`/`ne`)
`inline`. Its review flagged a pre-existing gap it deliberately did not fix, and
mis-attributed it to `check_combinator_cycles`. Two probe subagents (2026-08-27)
relocated the defect and killed the fix design the note implied. This slice is the
corrected version of that follow-up.

## The gap (measured, gdb-confirmed)

A **recursive `impl: Ord`** -- one whose `cmp` compares values of *its own type* with a
surface comparison, rather than delegating to its fields -- makes the compiler overflow
its own stack. `cargo run -- build` exits 134 (SIGABRT, `fatal runtime error: stack
overflow`) and **prints no diagnostic first**.

The unbounded recursion is at **lowering**, not check time:

```text
lower_call                (src/ir/func_builder/calls.rs:308)
  -> lower_resolved_word_call (calls.rs:226)   splices `cmp`'s body
     -> trait-call dispatch lookup (calls.rs:292)
        -> resolves back to the same type's own `cmp`
           -> lower_resolved_word_call ...     unbounded
```

Pre-existing: reachable at base via a user `inline` combinator calling `cmp`. P7.S8 moved
it onto the shipped library comparisons, so it is now reachable from any recursive-type
`impl: Ord`.

## What was refuted

**`check_combinator_cycles` cannot catch this, and must not be widened to try.**

- It runs pre-dispatch (`src/check.rs:778`, before any body is checked) over *surface
  callee names* matched against `word.name`. The cycle is not a syntactic fact: the edge
  back to the impl's own `cmp` exists only once dispatch resolves a bare `cmp` to a
  concrete type's member. The pass correctly finds nothing.
- The direct-self-call case it *could* catch is already handled elsewhere:
  `rewrite_member_self_calls` (`src/parser.rs:566`) rewrites a literal self-call inside an
  `impl:` member body to the synthesized name before the checker runs.
- **Widening it was built and measured, not merely argued against.** Adding an edge from a
  bare trait-member callee to every impl's resolved member (the data is available:
  `ImplDecl.resolved` is populated by `check_impl_decls` at `src/driver.rs:833`, before the
  cycle check) rejects the ordinary field-delegating impl:

  ```sooth
  impl: Ord for Point : cmp | a b | a Point> | ax ay | b Point> | bx by |
    ay drop by drop ax bx lt ~[ Less ] ~[ ax bx gt ~[ Greater ] ~[ Equal ] if ] if ;
  ```

  which builds and runs on `main`, with:

  ```text
  error: an always-spliced word cannot be recursive (the inliner would splice it forever):
  `lt` -> `cmp` (member of trait `Ord` for `Point`) -> `lt`
  ```

  Eight tests fail,
  including `a_concrete_impl_ord_delegating_to_lt_builds_and_runs` -- P7.S8's own R1c
  regression test, added to guard exactly that shape. The over-approximation is
  dispatch-blind, and dispatch on the operand type is the entire difference between the
  legitimate and pathological programs. Dead design; do not revisit.

## Scope

A **splice-depth budget** at the recursion site. Exceeding it emits a located diagnostic
naming the splice chain instead of recursing into a stack overflow.

Chosen because it is correct at the layer where the unbounded work actually happens, and
because it **cannot false-reject**: it bounds recursion rather than changing acceptance, so
no program that compiles today stops compiling. `FuncBuilder` already carries
`member_splice_depth` and `splice_uid_stack` at exactly this point (both introduced by
P7.S8), so the counter has a natural home.

## Open questions (settle these in the spec, with probes)

- **Which counter.** `member_splice_depth` is scoped to R1's own push/pop bracket and
  counts member re-splices specifically. Determine whether it is already the right monotone
  measure of nesting depth, or whether the guard needs its own total-splice-depth counter
  covering the ordinary combinator splice path too. Do not assume; read the bracket.
- **Where the guard sits.** `lower_resolved_word_call` is the confirmed recursion site. A
  mutual cycle of *ordinary* `inline` combinators is already rejected statically by
  `check_combinator_cycles`, so the dispatch-mediated path may be the only one needing a
  budget. Confirm whether one guard at the shared entry covers both, or whether that
  double-guards something already safe.
- **The budget value.** Must sit above any legitimate splice depth. Measure the real
  maximum across `lib/` and the test corpus (instrument the counter, run
  `cargo test --no-fail-fast`, take the max) and pick a budget with clear headroom above
  the observed ceiling, rather than a number chosen by feel.
- **Diagnostic content.** The useful message names the splice chain
  (`cmp` for `Wrap` -> `lt` -> `cmp` for `Wrap`), which needs a stack of
  (word name, span) alongside the depth counter; `splice_uid_stack` carries uids, not
  names. Decide whether the chain is worth that bookkeeping or whether a message naming
  only the outermost word and the depth limit is enough. Diagnostics are behaviour here
  (CLAUDE.md), so whichever is chosen gets a golden test.
- **Rendering.** An `impl:` member's internal name is the synthesized `cmp;Ord;0;Wrap`,
  which a user never wrote. The diagnostic must go through `render_word`
  (`src/resolve.rs:153`), not print the raw name.

## Out of scope

- Any change to `check_combinator_cycles`, `is_combinator`, or the P7.S8 uid rules. The
  first is refuted above; the latter two are confirmed working.
- Precise, type-directed cycle detection on resolved dispatch targets. That would buy a
  sharper message ("this is a cycle" rather than "this exceeded the splice budget") at the
  cost of a second graph at a new pipeline stage, interacting with the existing self-tail
  relaxation and overload over-approximation. Deferred deliberately: revisit only if the
  budget diagnostic proves confusing in practice.
- The other two P7.S8 follow-ups (unsatisfied-`Ord` attribution, REPL trait/impl
  accumulation). Unrelated; each is its own slice.

## Exit

`cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green. A recursive-type
`impl: Ord` whose `cmp` compares its own type produces a located diagnostic and a non-zero
exit, never SIGABRT, as a golden test. The field-delegating `impl: Ord for Point` above
still builds and runs, and P7.S8's `a_concrete_impl_ord_delegating_to_lt_builds_and_runs`
still passes -- the explicit no-false-rejection guard. The chosen budget is justified in
the spec by a measured maximum legitimate splice depth, not asserted.
