# Phase 3 Slice 4 — Generalized recursive disposal (brief)

Slice 3 gave a directly self-recursive type (`type: List | Nil | Cons v i64 next ^List ;`)
one fused destructor loop instead of recursive `cell_drop`/`struct_drop`/`enum_drop` calls,
so disposal runs in constant stack. That loop fires on exactly one shape: a field whose
type is `^Self`, checked by exact match. This slice widens *detection*, not the loop
mechanism itself, to three shapes Slice 3 named and deferred: the recursive edge going
through an intervening struct, through a nested cell, or through a different declared type
entirely. It does not touch branching (a node with more than one recursive edge): that stays
on the recursive path, moved to Phase 6 (see ROADMAP.md), because it needs a heap-allocated
worklist this slice has no reason to build.

Prerequisite state: Slice 3 is merged (`190cda6`), 679 tests green.

## Recon: what already works today (measured, not assumed)

Three probe programs, one per gap, built and run on `190cda6`:

1. **Indirect cycle through a wrapper struct.**
   `type: Wrap v i64 next ^List ; type: List | Nil | Cons w Wrap ;` — `List`'s recursive
   edge exists, but it's one struct hop away from the enum itself (`Cons` holds a `Wrap`
   value, and `Wrap` holds the `^List` cell). Compiles and runs today.
2. **`^^Self`.** `type: L | Nil | Cons n i64 next ^^L ;` — the recursive field's type is a
   cell of a cell of `L`, not `^L` directly. `recursive_loop_field`'s exact match
   (`cells.payload[c] == self_ty`, ir.rs:922) sees `OwnedCell(inner)`, not `L`, and misses
   it. Compiles and runs today.
3. **Multi-type (mutual) cycle.** `type: A | ANil | ACons x i64 next ^B ; type: B y i64 z
   ^A ;` — the cycle is length 2 across two declared types. (`A` needs a base case, `ANil`,
   or the pair is uninhabited: two bare structs holding cells of each other can never be
   built, confirmed during Slice 3's spec review.) Compiles and runs today.

All three, at small N (4), produce a **balanced alloc/free trace** under
`SOOTH_TRACE_ALLOC=1` — today's plain recursive disposal is already correct for all three
shapes. The only defect is depth:

| shape | N=50,000 @ 8MB | N=100,000 @ 8MB |
|---|---|---|
| wrapper-struct list | exit 0 | **SIGSEGV**, exit 139 |
| `^^Self` list | exit 0 | **SIGSEGV**, exit 139 |
| mutual A/B chain | exit 0 | **SIGSEGV**, exit 139 |

Identical to Slice 3's own pre-fix baseline (50k passes, 100k doesn't) — confirming none of
these three shapes hit the fused loop today, and the failure is the same recursion-depth
defect Slice 3 fixed for the direct case only.

**Finding 4: `begin_loop`/`finalize_loop` (ir.rs:1515, 1536) are not tied to one aggregate
type.** `begin_loop` takes `&[Value]` and derives each phi's type from `self.value_type(p)` at
   the call site; nothing hardcodes a single struct/enum shape. A loop body is free to do
   more than one unwrap-step per physical iteration — read node, extract the next cell,
   read *that* node, extract the next cell again, back-edge — as long as the value fed back
   on each back-edge matches the phi's type. This is why a multi-type or nested-cell cycle
   doesn't need new loop infrastructure, only a loop body that knows how many steps make up
   one trip around the cycle.

## Decided (locked, one at a time)

**D1. Scope is detection and loop-body generalization only; branching stays out.**
`recursive_loop_field`'s narrowing rule — loop the *last* recursive field, recurse any
others — carries forward unchanged. This slice widens what counts as "a recursive field"
(direct `^Self` today; indirect-through-struct, nested-cell, and cross-type cycles after
this slice) but never changes *how many* edges per node the loop touches. A node with two
or more recursive edges (of any kind, direct or newly-generalized) still loops the last and
recurses the rest, exactly as today. Worklist-based disposal for that case is Phase 6's, not
this slice's, because it needs a growable heap structure and a fallible-push story neither
of which this slice's gaps require.

**D2. No runtime cycles exist, so disposal never needs aliasing safety.** `^T` ownership is
exclusive (no `dup` on a cell, no borrow); struct/enum setters (`S<fi`) are `( S Ti -- S )`,
a whole-value functional transform, never a write through an existing pointer. There is no
way to construct a value whose recursive edge points back at an ancestor's already-built
cell. Every one of these three *type*-level cycles still produces a value-level *tree* at
runtime. The generalization needed is purely "recognize more shapes as loop-eligible and
emit a loop body that walks them," never "guard against visiting a node twice" — no
visited-set, no double-free guard, anywhere in this slice.

## Open questions the spec must answer

- **Detection mechanism and its home.** Slice 3's spec review flagged that the checker's
  own cycle graph (`type_node`/`visit_recursion`, check.rs) deliberately excludes `^` edges
  and produces no reusable component data; no SCC/reachability code exists anywhere in the
  repo. This slice needs its own pass over `Registries`, almost certainly in `ir.rs`
  alongside `recursive_loop_field`, walking struct-field / enum-variant / cell-payload edges
  to find the (necessarily unique, given D1's one-edge rule per node) path back to a type
  whose recursive field, followed through zero or more non-cell hops and one or more cell
  unwraps, reaches the enclosing type again.
- **What "the recursive edge" means once it can span types.** For the mutual case, *both*
  `A` and `B` get destructors, and `drop` can be called on a value of either type directly.
  Does each type's destructor get its own loop (starting its own unrolled walk from its own
  shape), or does only one direction get a loop and the other stays recursive (calling into
  the looped one)? This changes the shape of the codegen change materially and needs a
  decision, not an assumption.
- **How many hops a single loop iteration takes**, and whether that's fixed at codegen time
  per cycle (a length-2 cycle unrolls 2 steps per iteration; `^^Self` unrolls 2 cell-peels;
  a 3-type cycle would unroll 3) or generalized to an inner loop over an arbitrary chain
  length. Given D1 keeps this to simple cycles (no branching), a fixed per-shape unroll
  computed once at codegen time is almost certainly enough — worth stating as a decision
  rather than leaving implicit.
- **Whether wrapper-struct indirection composes with `^^Self` or multi-type cycles** (e.g. a
  wrapper struct in a 2-type cycle) — in scope by the general mechanism, or worth an explicit
  test even if untargeted, to make sure the detection pass doesn't special-case exactly the
  three probes above and miss the general case.

## Dogfood

Extend or add to `examples/list.sth`'s family: at minimum, one program per generalized
shape (wrapper-struct list, `^^Self` list, mutual A/B chain) built and disposed at a depth
that would segfault today (100k+) and pass under a constrained `ulimit -s`, the same
verification method Slice 3 used for its own constant-stack claim.
