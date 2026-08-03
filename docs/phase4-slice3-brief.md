# Phase 4 Slice 3: aggregate-return aliasing and the loop-carried copy (brief)

This slice took over slice 3's position from generic struct declarations (moved to
Phase 6) because slice 1's synthesized return bundles removed that slice's only named
consumer, and because this defect stopped being a corner case the moment slice 1 put
every multi-output word on the aggregate-return path. ROADMAP.md frames it as one
problem: an aggregate returned by value gets one QBE stack slot per call site, and a
self-tail-recursive word lowers to a loop rather than a new frame, so a result carried
across the back-edge points at storage the next iteration overwrites.

That framing is right. What it undersells is the severity, and what it leaves open is
the shape of the fix, which is more constrained than it looks. Both are measured below.

## Recon: what already exists (measured, not assumed)

**1. It is not a wrong-number bug. It corrupts destructor contents.** The known repro
prints a wrong integer, which reads as cosmetic. The linear-spine version does not.
With a destructor-carrying resource, disposing the previous iteration's value after the
current iteration's value has been produced:

```
type: Res n i64 ;
: drop ( Res -- ) | r | r Res>n 1000 + . ;
: mk ( i64 -- Res ) | n | n Res ;
: loop ( i64 Res -- Res )
  | n prev |
  n 0 = if prev else
    n mk | cur |
    prev drop
    n 1 - cur loop
  end ;
: main ( -- ) 3 0 mk loop drop ;
```

prints `1000 1002 1001 1001`; correct is `1000 1003 1002 1001`. `prev drop` disposes a
value whose bytes `mk` already overwrote. Disposal is still exactly-once, so the
checker's guarantee holds in form, but the *contents* disposed are the wrong ones: one
resource is disposed twice by content and another never. For 8b's actual use case, a
`Res` wrapping a file descriptor or a heap pointer, that is a double-close of one handle
and a leak of another, not a misprinted number. Reordering the body so `prev drop` runs
*before* `n mk` prints correctly, which pins the trigger precisely: the old value must
be read after the new call.

**2. It is type-agnostic across all three aggregate kinds.** `IrType`'s aggregates are
`Struct(StructId)`, `Enum(EnumId)`, and `Array(ArrayId)` (`src/ir.rs:76-112`), all
represented identically at runtime as a pointer to aggregate storage and spelled
`:S`/`:E`/`:A` in ABI positions. Struct is the known repro (`0 2 1 1`, correct
`0 3 2 1`). Array reproduces the same shape (`0 2 1`, correct `0 3 2`) via
`: mkarr ( i64 -- [i64 4] ) 4 fill ;` carried through the same loop. Enum follows by
construction, sharing the representation and the same return path. The fix is uniform
over the three or it is wrong for two of them.

**3. The mechanism is an interior pointer, not the call ABI.** `field_value` in
`src/ir.rs` hands back a pointer *into* the call's result slot rather than a copy, and
`begin_loop`/`finalize_loop` (`src/ir.rs:2259`/`:2280`) build the loop as a header block
of `Instr::Phi` whose back-edge arms are appended afterward. So the phi carries the
interior pointer, and the storage it points into is reused by the next iteration's call.
Non-tail and mutual recursion are unaffected, since those keep real frames: the same
program with a trailing term after the recursive call (making it non-tail) prints
correctly. The defect is specific to the self-tail-call loop transform.

**4. The checker already rejects the reference form of this, with a diagnostic.**
`check_reference_across_back_edge` (`src/check.rs:4041`) rejects a reference whose owned
root is a local of this frame when it crosses the back-edge:

```
error: a reference to a local cannot cross a loop in `loop` (line 11)
  a reference derived from `fresh`, a local of this frame, crosses the self-tail-call
  back-edge to `loop`: that local's storage does not survive to the next iteration
```

This is the single most scope-reducing fact in the recon. The fix has to handle
aggregate *values* crossing the edge and nothing else: it cannot silently invalidate a
reference, because no reference into frame-local storage is allowed across the edge in
the first place. A reference whose referent lives in an ancestor frame is legal and
unaffected (`: walk ( &!List -- )` in `examples/refs.sth`, explicitly preserved by that
check's own doc comment).

**5. The checker also already computes the crossing set, and guarantees it is moved.**
R15's `check_linear_across_back_edge` (`src/check.rs:4062`) rejects a linear value that
survives the back-edge stranded below the call's arguments or held in an unconsumed
local; a value *moved into* the call's arguments is forwarded and stays legal. So at the
back-edge the live aggregate set is exactly the recursive call's arguments
(`stack[base..]`), the same slice `check_reference_across_back_edge` scans. Lowering
needs the same set, and it is already identified on the checker side.

**6. No new IR instruction is needed.** `Alloc` (a frame-local aggregate slot, size and
align from the layout registry) and `Blit` (a byte copy between aggregate pointers)
already exist (`src/ir.rs:926-933`), and `Blit` is already the mechanism behind the
byte-copy `dup`, a setter's copy-all, and a nested-struct field store. So this is a
lowering-shape question with no backend-neutrality decision inside it and no new backend
surface. Note `Alloc` emits inline with no hoisting (`src/backend/qbe.rs:1029`), which
is what kills one of the two obvious fixes.

**7. A two-aggregate swap across the back-edge works correctly today.** A loop carrying
`a` and `b` and recursing with `b a` prints the correct alternation and returns the
correct pair, because pointer-phi shuffling is fine when neither aggregate is re-produced
inside the loop. This matters because the plausible fix breaks it: blitting into stable
slots on the back-edge, done naively in order, writes A's slot from B and then B's slot
from the already-overwritten A. The fix inherits the classic parallel-copy problem, and
there is a working program in hand that catches it.

## Why the obvious fix is dead

Copying the call result into a fresh slot at the call site fails in both placements
available to it. Hoisted to the entry block it is a single slot again and the bug returns
unchanged. Left in the loop body it is a stack bump per iteration, since QBE's `alloc`
emits inline (recon 6), which breaks the constant-stack iteration guarantee this phase
exists to demonstrate, in the exact feature (`each`/`fold`) meant to demonstrate it.

So the copy has to land on the back-edge, into a slot that is stable across iterations
and allocated once in the entry block. That turns the fix from a call-site rewrite into
phi elimination for aggregates, which is where recon 7's cycle problem comes from.

## Decisions the spec has to make

1. **Which values get a stable slot.** The narrow rule is every aggregate-typed argument
   of a self-tail recursive call (recon 5 gives the set). The narrower one is only those
   that can alias a call result produced in the loop body, which is fewer copies but
   needs an aliasing analysis that does not exist yet. Recommendation: take the broad
   rule. A blit per carried aggregate per iteration is cheap next to the analysis, and
   the broad rule is checkable by inspection, which matters more here than the copies.

2. **Cycle breaking on the back-edge.** Whatever placement is chosen must keep recon 7's
   swap program correct. Routing every back-edge blit through a temp is the simple
   answer and costs a second copy; detecting cycles and breaking only those is the
   cheaper one. The spec should pick explicitly rather than let the implementation
   discover the swap case in review, and the swap program should be a criterion.

3. **What happens to the phi.** If a carried aggregate always reads from its stable slot,
   the header phi for it is degenerate (the same value from every predecessor) and the
   aggregate is no longer really SSA-carried at all. Deleting it is cleaner than leaving
   a phi that always selects the same pointer; the spec should say which, because it
   decides whether `begin_loop` stops emitting phis for aggregate params or
   `finalize_loop` rewrites them.

4. **Where the initial value comes from.** The entry arm needs the incoming argument
   blitted into the stable slot once before the loop starts, or the first iteration reads
   uninitialized storage. This is easy to miss because the existing repros all survive
   iteration 1.

5. **Destructor interaction.** Recon 1 is the acceptance bar: disposal must remain
   exactly-once *and* dispose the right contents. The spec should state that the blit is
   a move (the source is dead after it, guaranteed by R15) so no second live copy is
   created, and make the destructor-carrying program a criterion rather than reasoning
   about it abstractly.

6. **Whether non-aggregate carried values change at all.** They should not: scalars are
   copied out by value and are already correct. The spec should say so, so the
   implementation does not generalize the fix into a rewrite of loop lowering.

## Scope

In: aggregate values (`Struct`/`Enum`/`Array`) crossing a self-tail-call back-edge, the
stable-slot allocation and back-edge copy in lowering, cycle breaking, and the goldens
for all of it. Out: quotations and combinators (slices 4 and 5), non-tail and mutual
recursion (real frames, unaffected, recon 3), references across the back-edge (already
rejected, recon 4), and any change to the call ABI itself, which is correct as it stands.

## Exit

The struct, array, and destructor repro programs above all print their correct sequences;
the two-aggregate swap program still prints its correct alternation; and a constant-stack
check confirms the fix did not introduce a per-iteration stack bump. Goldens for each in
the native test suite, including the destructor case as the resource-safety witness
rather than only the printed-value cases.
