# Phase 3 Slice 3 — Recursive heap data + `isize` (as built)

From [`phase3-slice3-brief.md`](./phase3-slice3-brief.md). Base: `main` at Slice 2, 654 tests
green. Delivered across 10 commits (`b0f6883` … `6cd5615`).

## Problem statement

Recon on the base commit found every recursive shape already building and disposing
correctly with balanced traces: recursive enums, recursive structs, binary trees, mutually
recursive types, the wrapper-struct list, and `^^Self`. `type_node` already excluded
`OwnedCell` from the by-value cycle graph (incidentally, and untested), and by-value
recursion was already rejected. **Destructor stack depth was the one real defect**: a
100,000-node list segfaulted under a 1 MB and an 8 MB stack alike, only clearing at 64 MB.
So most of this slice pins already-correct behaviour with tests rather than building it; the
actual new work is the fused destructor loop that fixes the depth defect, plus `isize`
(cheap to add because it mirrors `usize`, though it ships with no consumer this slice).

## What shipped

**`isize`** — `Type::Isize` / `IrType::Isize`, a signed target-width mirror of `usize`;
mixing the two is a plain type mismatch, and printing routes to `$fmt`, not `$ufmt`.
`norm_scalar` became a wrapper over `norm_scalar_ww(ty, word_width)`, so neither size type
carries a literal `64`.

**Recursion rule** — pinned, not changed: a type cycle is legal iff every cycle passes
through at least one `^`, in struct-field and enum-variant position alike. `type_node` now
excludes `OwnedCell` deliberately, with a comment, instead of by fall-through. No
uninhabitedness detection; array elements stay by-value edges, so `[^T N]` is still rejected
by Slice 2's linear-element rule. The by-value cycle diagnostic is unchanged: a bare string,
no span, unbackticked names — a deliberate carve-out, not an oversight.

**Disposal ordering** — reversed globally: every owning cell frees its block *before*
dropping the copied-out payload, and disposal is pre-order (a node's own fields drop and its
cell frees before descending). Sound because `load_owned_payload` copies the payload out
before the block is touched, for every shape including nested `^^T`. This also inverted two
Slice 2 goldens and their doc comments, since Slice 2 had shipped the opposite order.

**Fused iterative destructor** — the core new mechanism. `recursive_loop_field` is a fresh
pass over `Registries` rather than a reuse of the checker's cycle graph, which cuts `^` edges
entirely instead of detecting them: a field of type `^T` is a recursive edge iff the cell's
payload is the enclosing type itself. A directly self-recursive struct or enum gets one loop
via `begin_loop`/`finalize_loop`; `emit_recursive_step` handles the recursive edge, and any
non-looped edges keep an ordinary recursive drop call.

The load-bearing invariant is copyout ordering: **every read of the current node must be
emitted before the copyout that overwrites the reused, entry-hoisted frame slot.** This can't
be caught by counting allocs and frees — the loop still allocates and frees exactly the right
number of times even if a field read happens after the copyout, it just reads garbage (a
stale or repeated node's data) while the trace stays perfectly balanced. The suite only
catches a violation by tagging every node distinctly and asserting the full ordered
transcript, never a count.

The trailing `seal_block` is guarded on whether the block already terminated, so a
self-recursive struct's exit-less loop (the shape is uninhabited — `^` is non-null, so
building one needs one first — but a destructor is still synthesized for every declared
type) doesn't emit a duplicate block label, which QBE would reject outright. Multi-child
types (a tree node with two `^` fields) loop on the **last** declared recursive field and
recurse the others via the ordinary path; looping the last child rather than the first is
what makes a right-leaning shape constant-stack while a left-leaning one stays O(depth).
Mutually recursive types stay on the recursive path entirely — no fused loop spans a
multi-type cycle. Non-recursive destructors keep straight-line synthesis.

**Allocation failure** — trap stays; no allocator code touched. `ROADMAP.md` no longer
claims this slice introduces optional/non-null pointers; those move to Phase 4's generics.

## Documented limitation

Constant-stack disposal is guaranteed only for directly self-recursive types. Indirect
cycles through an intervening struct, `^^Self` (excluded by construction — the phi type
wouldn't match), left-leaning/non-loop children, mutually recursive types, and the
non-direct cycles of a mixed type are all still O(depth), verified as such rather than
assumed.

## Implementation

1. **`isize`** (`b0f6883`, `50996e0`, `35c28bc`) — the scalar type across `ast.rs`/`ir.rs`/
   `backend/qbe.rs`, then generalizing the `usize`-named coercion/diagnostic plumbing in
   `check.rs` (parameterised rather than duplicated: `is_size_type`,
   `SlotMatch::{LiteralSizeType, NeedsSizeConversion}`, `PairMatch::NeedsSizeConversion(Type)`,
   `size_conversion_needed_error(target)`), and hardening three QBE signedness guards
   (`Shl`/`Shr`, `Rem`, `Cmp`) that had been trusting `IrType::Usize` directly instead of
   normalizing first.
2. **Recursion rule, pinned with tests** (`c597cef`, `2218465`, `5028847`) — `type_node`'s
   `OwnedCell` exclusion made explicit (an exhaustive match, not a fall-through arm), plus the
   by-value-cycle/`^`-cycle/array-element tests the recon above already found true.
3. **Disposal order reversed** (`0d3bb76`, `112f8da`) — `synthesize_cell_destructor` frees
   before dropping the copied-out payload; the two affected Slice 2 goldens, their doc
   comments, and the ROADMAP wording describing the old order were updated together.
4. **Fused iterative destructors** (`a2f04a8`) — `recursive_loop_field`, the loop-fused
   `synthesize_struct_destructor`/`synthesize_enum_destructor`, and
   `FuncBuilder::emit_recursive_step`/`finalize_loop`.
5. **Slice completion** (`6cd5615`) — pre-order disposal goldens, the tree/multi-child/
   mutual/constant-stack goldens, `examples/list.sth`, and the ROADMAP rewrite.

## Enduring criteria

The goldens live in `tests/phase0.rs`, `tests/phase1.rs`, and `src/check.rs`; this is what
makes them trustworthy rather than a list of names:

- **`isize` round-trips and diverges correctly from `usize`**: declares, computes, prints
  signed, converts to/from `i64`; both size types derive their width from the `word_width`
  parameter, never a hardcoded `64`; mixing the two, or leaving a computed value where a
  declared `isize` output is expected, is an error naming the backticked type.
- **The recursion rule**: a by-value self- or mutual cycle is still rejected, naming the
  full path; a `^` edge breaks the cycle in both struct-field and enum-variant position; a
  `^` edge inside an array element does *not* count, since arrays are Slice 2's
  linear-element territory, not the recursion rule's.
- **Disposal ordering and the copyout hazard**: every disposal is pre-order, and every
  owning cell frees before its payload drops — proved with distinct per-node tags asserted
  as a full ordered transcript, since a plain alloc/free count cannot distinguish correct
  disposal from the copyout-hazard bug (a stale or duplicated node's data read after the
  slot was overwritten, with the trace still perfectly balanced).
- **The detection pass doesn't false-fire**: three near-miss shapes (a cell of a *different*
  aggregate, `^^Self` where the inner payload is a cell rather than the enclosing type, a
  cell of an unrelated enum) all keep straight-line synthesis, verified by both trace and
  size.
- **Constant-stack disposal is proven, not asserted**: a 1,000,000-node list and a
  1,000,000-node right-leaning tree both dispose under a 1 MB stack (`ulimit -s 1024`), exit
  0. These two are the real proof that the loop exists — a small fixed-size tree whose tags
  come out in the right order only proves the loop descends the *last* field, not that
  looping happens at all; a compiler with no fused loop whatsoever would still pass that
  test. The two 1M-node, 1 MB-stack goldens are the ones a non-fused compiler cannot pass.
- **The pre-change compiler actually fails this**: run against the base commit (before the
  fused loop), the 1M-node list golden segfaults at a 1 MB, 8 MB, *and* 64 MB stack alike,
  so the constant-stack criteria discharge a real defect rather than passing vacuously.
- **The boundary is real, not silent**: mutually recursive types and other indirect shapes
  stay on the recursive path and its depth limit — asserted as an asymmetry (a small chain
  disposes correctly; a 300k-node chain overflows a 1 MB stack the same way the pre-fix list
  did) rather than merely documented.
- **No regression**: all 14 pre-existing examples stay byte-identical, plus
  `examples/list.sth` (builds 10 nodes, consumes 3 via `pop`/`sum-first`, prints `6`, drops
  the remaining 7 through the fused loop) and a REPL `:quit` disposing a residual recursive
  value.

## Deviations from the spec

- The `usize`-named coercion plumbing was parameterised, not duplicated (Implementation,
  item 1).
- `examples/list.sth` builds 10 nodes, consumes 3, prints `6`, then drops the remaining 7
  through the loop; the brief left the exact shape of the dogfood open.

## Coverage gaps, stated

The recursive-edge detection pass (`recursive_loop_field`) has no test that targets it
directly; the near-miss golden above is its proxy. There is still no OOM-trap runtime test
anywhere in the suite; the `LD_PRELOAD` technique remains recorded but unwritten, so
"allocator unchanged" rests on the example sweep.

## Out of scope (unchanged)

Fused loops for indirect recursion and `^^Self` (the natural follow-on). Worklist disposal
for branching structures. Fused loops over multi-type cycles. Compiler-provided
`Option`/nullable pointers and returning allocation failure (Phase 4 generics). Pointer
arithmetic and differences. Zippers. Second-class refs (Slice 4), refcounting (Slice 5),
user destructor bodies (Slice 6), `Vec` (Phase 6).
