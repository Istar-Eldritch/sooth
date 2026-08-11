# Phase 6 — the surviving-set forwarding gate

Slice 10a, phase 6 of 7. Delivers **R14**.

## What landed

`back_edge_outs` (`src/check.rs:7157`) now forwards the `surviving` capture set from a
carried input to its ground output, following the bottom-aligned `index_map` built by
phase 5's `back_edge_declared_shape`. This closes the gap R14 names: the pre-rewrite
block built every output as a bare `Slot::computed(ty)`, which sets `surviving: None`
unconditionally, so an aggregate riding an erased quotation across a self-tail back-edge
silently lost its escape obligation — the same class of bug `d1b3f0a`/`bee407c` fixed on
every other value path.

Phase 5 had already landed the extraction (R14a) and the white-box test `#[ignore]`d.
Phase 6's only change is: un-ignore
`back_edge_outs_forwards_surviving_set_along_index_map`, and add the one line that makes
it pass:

```rust
if let Some(src) = index_map.get(i).copied().flatten() {
    out.surviving = carried_inputs[src].surviving;
}
```

## R14's five clauses, checked

1. **Named, callable function** — `back_edge_outs`, extracted in phase 5, called from the
   real back-edge arm at `check.rs:8799` with the actual caller slots `stack[base..]`, not
   a test-only stub.
2. **Aggregate witness** — the test's carried slot is `Type::Struct(.., "Agg")` with
   `surviving: Some(SurvivingCaptureSetId(0))`, `quot: None`.
3. **Non-`None` map entry** — `index_map = vec![Some(0)]`, so the witness exercises the
   forward, not the untouched-slot fallthrough (a shape whose map is all-`None` would make
   every assertion vacuous).
4. **Direct assertion, bypassing `union_surviving`** — the test calls `back_edge_outs`
   directly and asserts `outs[0].surviving == Some(set)` before any join runs, so
   `union_surviving` (`check.rs:844`) has no chance to mask a dropped forward by
   reconstructing the set from a sibling `if` arm.
5. **Un-ignored, and passing.**

## R20 — mutation evidence

Reverted the forward at `check.rs:7169` (`out.surviving = carried_inputs[src].surviving;`
→ `let _ = src;`, leaving `surviving: None` from `Slot::computed`), ran the single test,
then restored via `diff` against the pre-mutation copy (byte-identical):

```
test check::tests::back_edge_outs_forwards_surviving_set_along_index_map ... FAILED

thread '...' panicked at src/check.rs:15604:9:
assertion `left == right` failed: the aggregate's surviving capture set must ride across the back-edge
  left: None
 right: Some(SurvivingCaptureSetId(0))
```

Restored; `git diff --stat` shows only the intended lines changed.

## Review fix folded into this phase

Round-1 review flagged `out.quot = carried_inputs[src].quot` (R14's other named field) as
dead code: the call site (`check.rs:8797`) filters `carried_inputs` to `s.quot.is_none()`
before it reaches `back_edge_outs`, and `Slot::computed` already produces `quot: None`, so
the assignment provably always writes `None` into `None`. Confirmed by mutation (deleting
it breaks no test). Removed the line and trimmed the function's doc comment to describe
only `surviving`, which is the sole field that can ever ride across this path — a bare
erased quotation is filtered out upstream as non-phi loop state before `back_edge_outs`
ever sees it.

## Green

`cargo fmt --check && cargo clippy -- -D warnings && cargo test`: 784 lib tests (0
ignored), full integration suite including `qbe_baseline` (byte-identical). `git diff
--stat` touches only `src/check.rs` (the forward's doc comment and the dead `quot` line).
