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

A **splice-depth budget** at the recursion site, plus the error path needed to surface it.
Exceeding the budget produces a located diagnostic instead of recursing into a stack
overflow.

The budget cannot false-reject: it bounds recursion rather than changing acceptance, so no
program that compiles today stops compiling.

Two parts, and the second is the larger one:

1. **The guard.** A depth check in `lower_resolved_word_call`, using the existing
   `member_splice_depth`.
2. **A fallible lowering path.** `FuncBuilder` has no way to report an error at all today,
   so the diagnostic cannot reach the user without one. See "the real cost" below.

## Settled by probes (2026-08-27)

- **Counter: reuse `member_splice_depth`. No new counter.** Its bracket
  (`src/ir/func_builder/calls.rs:214-236`) has exactly one statement between `+= 1` and
  `-= 1` -- no `?`, no early return -- so it is balanced on every reachable path.
  `lower_resolved_word_call` is the sole definition and all three call sites (`:294`,
  `:308`, `:332`) sit inside `lower_call`: a single choke point every dispatch-mediated
  splice funnels through, whichever of `trait_calls`/`splice_trait_calls`/
  `builtin_overloads` resolved it. It does not count the ordinary combinator splice hops
  (the `lt` legs of the chain), which is correct rather than a gap: that leg is statically
  acyclic, so re-entries here are the only quantity that can grow without bound.
- **Guard site: `lower_resolved_word_call` only.** The ordinary `inline` combinator splice
  (`calls.rs:665`) is separate code -- its own uid minting, its own alpha-rename -- and is
  already proven finite pre-lowering by `check_combinator_cycles`. Guarding it too would
  validate a scenario that cannot happen.
- **Budget: 64.** Measured maximum legitimate `member_splice_depth` across the whole corpus
  (full test suite, `examples/`, `lib/`) is **2**. The pathological case overflows at depth
  **148** on a 2MB test-thread stack and **601** on the default 8MB main stack. 64 sits ~30x
  above anything legitimate and fires well before the stack dies even in the tighter
  test-thread case. The gap is wide in both directions; this is not a finely balanced
  number.
- **Span: capture the outermost one only.** `alpha_rename_member_locals` copies
  `term.span` verbatim (`src/ast.rs`, `rename_terms`), so spans *mix by depth*: the
  outermost frame is the user's real call site, but deeper frames point into
  `lib/cmp.sth`'s own bodies -- library source the user did not write and cannot fix. The
  diagnostic must point at the user's call site, always.
- **Rendering: `render_word` (`src/resolve.rs:158`, `pub(crate)`).** It turns
  `cmp;Ord;0;Wrap` into ``` `cmp` (member of trait `Ord` for `Wrap`) ```. Already called
  cross-module from `src/check.rs`, and `src/ir/driver.rs` already calls into
  `crate::resolve`, so there is no visibility barrier. Its output is pre-delimited: the
  caller must not wrap it in further backticks.
- **House style template: `combinator_cycle_error`** (`src/check/combinators.rs:291-303`).
  The new message should read as its sibling.
- **Double-prefix hazard is live.** `main.rs` does `eprintln!("error: {e}")` while existing
  diagnostics such as `combinator_cycle_error` also embed `error: ` themselves. Decide the
  new message's prefix deliberately and pin it in the golden test.

## The real cost: lowering has no error path

The counter is free; surfacing it is not, and an earlier draft of this brief undersold
this.

Every function in the recursive lowering chain -- `lower_terms`, `lower_term`,
`lower_call`, `lower_resolved_word_call`, `lower_self_tail_combinator`, `lower_poly_call`
-- returns `()`. Nothing in `FuncBuilder` (`src/ir/func_builder/`, ~7200 lines, 205
functions) carries any error-signalling field or fallible path. The `.expect()` convention
throughout means "the checker guaranteed this", and `main.rs` installs no panic hook, so a
panic surfaces as raw Rust output rather than a compiler diagnostic. The only fallible
boundary is `pub fn lower(module: &Module) -> Result<IrModule, String>`
(`src/ir/driver.rs:10`), which propagates by ordinary `?` to `main.rs`.

**Decision: thread `Result` through the lowering tree** so the diagnostic reaches the user
the same way every other diagnostic in this compiler does. This is a mechanical refactor
across most of `func_builder/` and is deliberately in scope; the slice is a small guard
plus that refactor, not a small guard alone.

The alternative -- a typed panic payload caught by `catch_unwind` at `lower()`'s entry --
was considered and rejected. It buys a smaller diff by introducing panic-based control flow
into a compiler that deliberately has none (grepped: zero `catch_unwind` in the codebase),
leaving a new convention for the next lowering-time diagnostic to inherit.

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
exit, never SIGABRT, as a golden test pinning the exact message text. The diagnostic points
at the user's call site, not into `lib/cmp.sth`. The field-delegating `impl: Ord for Point`
above still builds and runs, and P7.S8's
`a_concrete_impl_ord_delegating_to_lt_builds_and_runs` still passes -- the explicit
no-false-rejection guard. No `.expect()`/panic remains on the path the budget guards, and
the `Result` threading leaves no lowering error silently discarded.
