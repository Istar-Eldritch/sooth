# Phase 4 Slice 6d: nested constant-stack loops (brief)

Today any `times` reached while a loop is already open is rejected: "a `times` cannot be
nested in a loop yet: nested constant-stack loops need a hoist-target split deferred to a
later slice". That limit is not hypothetical and not confined to a future feature. It bites
every combinator 6a shipped, because each drives its own `times` internally, so no
combinator composes inside a loop. `2 [ drop mk [ . ] c::each ] times` is a hard error on
`main`.

ROADMAP.md's 6d entry diagnoses this as `FuncBuilder::entry_block` doing two jobs (alloca
home and loop preheader) and prescribes "split the field". **Probing against the built
compiler sharpens that on three counts**, and one of them shrinks the slice:

- the two roles are *not* symmetric. The preheader role is already correct at any nesting
  depth; only the alloca role is wrong. `entry_block` keeps its job, `push_alloc` loses its
  assumption.
- there is no second loop mechanism to unify. All four loop sites already share one
  `begin_loop`/`finalize_loop` pair, so the fix lands once and every user inherits it.
- the failure is proportional to the **outer** loop's trip count, not the product of the
  two. The obvious "run a million iterations" probe passes while the bug is live.

Everything below was run, not read. Probes live in `/tmp/p6d/`; the mutated-compiler
experiments used a throwaway copy at `/tmp/sooth-6d`, since editing the checked-out tree in
place is what burned a reviewer last slice.

## Recon: measured against the built compiler

**1. There is one loop mechanism, not several.** All four loop-forming sites call the same
`begin_loop` (`src/ir.rs:2419`) / `finalize_loop` (`:2466`) pair: the whole-word self-tail
transform (`:2068`), the self-tail combinator splice that 6b's `while` uses (`:2628`, which
is what D8 chose over specializing), the `times` arm (`:2737`), and the generated destructor
loops (`:1568`, `:1645`, with aggregate staging off). Any fix to hoist placement is
therefore a single-site fix that all four inherit. Nothing needs merging first.

**2. The limit is per-`FuncBuilder`, not fundamental: nested constant-stack loops already
run today.** A self-tail-recursive *word* called from inside a `times` body compiles and
runs, because the callee is its own `IrFunc` with its own entry block:

```
: countdown ( i64 -- i64 ) dup 0 > if 1 - countdown else end ;
: main ( -- ) 3 [ drop 5 countdown . ] times ;
→ 0 0 0
```

So the slice is not inventing nested loops. It is removing the reason two loops cannot share
one frame, with a working reference already in-tree to compare against.

**3. With both guards removed, nested loops compute the right answers.** In a throwaway copy
with the checker guard and lowering's matching `debug_assert` deleted, `times`-in-`times`,
`times`-in-`times`-with-an-inner-allocation, and an aggregate carried across the outer loop
all produced correct output. The defect is not a miscompile of values; it is the
constant-stack guarantee, which fails silently until the process dies.

**4. The failure is severe, and scales with the *outer* trip count.** An inner-loop
allocation is hoisted once per *outer* iteration, so frame growth is
`outer_iterations × hoisted_bytes`, independent of the inner count. 1000 × 1000 iterations
allocating a `[i64 4]` runs fine in a 1 MB stack and looks green. 200,000 outer iterations
allocating a `[i64 32]` segfaults on the **default 8 MB** stack:

```
: main ( -- ) 200000 [ drop 2 [ drop 0 32 fill | a | a drop ] times ] times 99 . ;
→ Segmentation fault (exit 139), at both `ulimit -s 1024` and the 8 MB default
```

This is the single most important fact for the exit criterion: a nested-loop test with a
large inner count and a small outer count passes while the bug is fully live.

**5. Only the alloca role is broken. Seeding is already correct at depth.** `begin_loop`
sets `entry_block` to the block current when the loop opens, i.e. the loop's preheader. For
the *seeding blit* that is exactly right at any nesting depth: once per entry to that loop.
Verified by re-entering an inner loop that carries an aggregate accumulator:

```
: main ( -- )
  2 [ drop 0 4 fill 3 [ drop | a | &!a 0 >usize &!> 1 +! a ] times
      | b | &b 0 >usize &> @ . b drop ] times ;
→ 3 3      (not 3 6: the inner loop re-seeds per outer iteration, as it should)
```

ROADMAP's "split the field into an invariant alloca home and a per-loop preheader" implies
both roles move. Only one does.

**6. The precondition `push_alloc` silently relies on.** `push_alloc` (`src/ir.rs:2370`)
routes every hoisted `Alloc` into `self.entry_block`. That is correct only while the
preheader is reached once per call. For the whole-word transform it always is, because
`begin_loop` runs immediately after the params are minted, so the current block *is* the
function entry and the two roles coincide by accident. For a mid-body loop the preheader is
an ordinary interior block, and the assumption breaks the moment that block sits inside
another loop. The field's doc comment (`:2251`) states the alloca-home intent carefully and
never mentions the assumption.

**7. The two roles are welded together at one call site.** `begin_loop` emits *both* the
carried aggregate's stable-slot allocation (`alloc_aggregate`, which routes through
`push_alloc`) and its seeding blit (`push_alloc(Instr::Blit(p, stable, size))`,
`src/ir.rs:2440`) through the same helper, so they cannot currently land in different
blocks. Separating those two is the slice. It is also, per ROADMAP's own warning, the
slice-3 aggregate-staging code, so its guards want mutation-testing rather than a green run.

**8. The save/restore discipline already exists, duplicated.** Both mid-body call sites save
and restore four fields around a nested region (`src/ir.rs:2595`–`2641` for the combinator
splice, `:2718`–`2795` for `times`), and a unit test pins it
(`times_saves_and_restores_loop_state`, `:4681`). Nothing needs adding there; 6d adds a
fifth field to both, which is the argument for collapsing the dance into one helper.

**9. The rejection has two checker call sites and one lowering assert, all sharing one
message.** `src/check.rs:5985` fires at a `times` term (R18); `src/check.rs:5252` fires when
a self-tail combinator is spliced while a loop is open (6b's R14a); both call
`times_nested_in_loop_error` (`:5551`). Lowering carries a matching
`debug_assert!(self.header.is_none(), ...)` (`src/ir.rs:2710`). Deleting the checker guard
alone panics lowering, so all three move together.

**10. A diagnostic defect ships today.** Because R14a reuses `times_nested_in_loop_error`
verbatim, a `while` nested in a `while` reports "a `times` cannot be nested in a loop yet"
for a program containing no `times` at all:

```
import: c ".../lib/combinators.sth" ;
: main ( -- ) 0 [ dup 3 < if 0 [ dup 2 < if 1 + true else false end ] c::while drop
                           1 + true else false end ] c::while . ;
→ error: a `times` cannot be nested in a loop yet in `main` (line 4)
```

6b's tests assert only the substring `"nested in a loop"`, so the wording is not locked in
by a test and is free to change or disappear.

## Decided (locked, one at a time)

**D1. 6d adds no new loop lowering.** Forced by recon 1, not chosen: the mechanism is
already shared by all four loop sites. Any proposal that introduces a second lowering path,
or outlines a loop body into its own function to get a fresh frame, is rejected on the same
grounds 6b rejected specialization: it reopens 6a's "inlining is total" invariant and buys
nothing, since the hoist fix is one call site.

**D2. `entry_block` keeps its meaning; the new field is the alloca home.** From recon 5 and
6: the preheader role is correct at depth, so the fix is to give `push_alloc` a separately
tracked, invariant, per-function alloca home and leave `entry_block` alone. This is narrower
than ROADMAP's "split the field" and inverts which half moves. ROADMAP's 6d entry should be
corrected to match rather than quietly diverged from.

**D3. `begin_loop`'s two emissions are separated explicitly.** From recon 7: the stable-slot
`Alloc` goes to the invariant alloca home, the seeding `Blit` stays in the preheader. This
is the whole behavioural change, and it is the one place where getting it backwards
reintroduces the slice-3 aliasing class of bug rather than a stack-growth bug, so it wants a
test that fails when the blit is hoisted too far (recon 5's `3 3` / `3 6` probe is exactly
that test).

**D4. The four-field save/restore becomes one helper.** Justified by 6d adding a fifth field
to a dance duplicated verbatim at two call sites (recon 8), not as free cleanup. It removes
the "added the field to one site and not the other" failure mode, which is the realistic way
this slice regresses.

**D5. The constant-stack criterion is a bounded-stack test with a large *outer* count.**
From recon 4: the natural test shape (large inner count) passes while the bug is live. The
criterion must drive the outer loop and run under a constrained `ulimit -s`, following 6b's
`while_and_hand_threaded_loop_agree_across_stack_limits` precedent, and must be shown to
fail with the alloca home pointed back at `entry_block`.

## Open questions the spec must answer

**Q1. Does the rejection disappear or narrow?** After the hoist fix, is any shape still
worth rejecting, or do both checker call sites, the error function, and lowering's
`debug_assert` all get deleted? If any rejection survives, its message must name the
construct actually at fault (recon 10), and R14a must stop borrowing the `times` wording.

**Q2. Which nestings must work at exit?** ROADMAP claims 6d lifts the limit "for all five
combinators at once", which means `times`-in-`times`, `while`-in-`times`, `times`-in-`while`,
`while`-in-`while`, and a combinator inside a `times` all have to run. 6b shipped R14a/R14b
specifically to reject two of those. The spec should enumerate the matrix and say which cells
are criteria and which are merely no longer rejected.

**Q3. Is depth bounded?** Expectation, not measured: arbitrary depth falls out, because the
alloca home is per-function and the preheader save/restore already nests by recursion. The
spec should either verify a three-deep nesting or state the bound explicitly.

**Q4. Does the destructor-loop path need anything?** `src/ir.rs:1568`/`1645` open loops with
`stage_aggregates: false`. They presumably inherit D2 for free, but a generated destructor
running inside a user loop is a nesting case nobody has probed.

**Q5. What is the dogfood?** 6a's combinators composing inside a `times` is the obvious
candidate (it is the motivating example in ROADMAP's own entry) and gives a matched
hand-threaded twin, as `filter_while.sth` did for 6b.

## Out of scope

- **The polymorphic-`if` gap (6e)** and the quotation-in-a-polymorphic-body rejection
  (slice 7). Neither is touched here.
- **Re-opening D8.** The self-tail combinator loop stays a splice-time back-edge.
- **Quotation-taking words at the REPL (6c).** Independent and orderable either way.
- **Any change to what `times` means to a user.** This slice lifts a limit; it adds no
  surface syntax and no new loop form.
