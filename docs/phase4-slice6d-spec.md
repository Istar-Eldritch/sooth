---

# Phase 4 Slice 6d: nested constant-stack loops (the hoist-target split)

**Status: spec.** Base `main` @ `ecf0452`. Depends on 6a (inliner, `lib/combinators.sth`) for its consumers; independent of 6b/6c and orderable against them. Decisions D1–D5 locked (from `docs/phase4-slice6d-brief.md`, every claim of which was run against the built compiler, not read off ROADMAP).

Today any `times` reached while a loop is already open is rejected: "a `times` cannot be nested in a loop yet". The limit bites **every** combinator 6a shipped, because each drives its own `times` internally, so no combinator composes inside a loop (`2 [ drop mk [ . ] c::each ] times` is a hard error on `main`). This slice lifts that limit for all five combinators at once by fixing the one field that does two jobs.

The cause is narrow and measured. `FuncBuilder::entry_block` (`src/ir.rs:2251`) is simultaneously the **alloca home** (where `push_alloc` hoists every `Alloc`, so QBE's frame-bumping `alloc*` runs once per call rather than once per iteration) and the **loop preheader** (where a carried aggregate's seeding `Blit` runs, once per entry to that loop). Those two blocks coincide at exactly one loop level and diverge the moment loops nest. The brief's probing sharpens ROADMAP's "split the field" on one count that **inverts** it: only the alloca role is wrong. The preheader role is already correct at any depth (recon 5). So `entry_block` keeps its meaning; the new field is the invariant alloca home, and `push_alloc` loses its assumption.

## Locked decisions

- **D1.** 6d adds no new loop lowering. All four loop sites already share one `begin_loop`/`finalize_loop` pair (recon 1: the whole-word self-tail transform, the self-tail combinator splice, the `times` arm, the two generated destructor loops), so the fix lands once and every user inherits it. Any proposal that introduces a second lowering path or outlines a loop body into its own frame is rejected on 6b's grounds: it reopens 6a's "inlining is total" and buys nothing.
- **D2.** `entry_block` keeps its meaning (the per-loop preheader, correct at depth). The new field is a separately tracked, **invariant** per-function alloca home. This is narrower than ROADMAP's "split the field" and inverts which half moves.
- **D3.** `begin_loop`'s two emissions are separated explicitly: the stable-slot `Alloc` goes to the invariant alloca home; the seeding `Blit` stays in the preheader (`entry_block`). Getting this backwards reintroduces the slice-3 **aliasing** class of bug (over-hoisting the blit), not a stack-growth bug, so it wants a test that fails when the blit is hoisted too far.
- **D4.** The four-field save/restore duplicated verbatim at the two mid-body call sites (`src/ir.rs:2595`–`2641`, `:2718`–`2795`) collapses into one shared helper. Justified because 6d adds the alloca-home field to the saved set (now five), and the realistic way this slice regresses is "added the field at one site, forgot the other".
- **D5.** The constant-stack criterion is a bounded-stack test with a **large outer** count (recon 4: frame growth is `outer_iterations × hoisted_bytes`, so the natural large-inner shape passes while the bug is fully live). It runs under a constrained `ulimit -s`, following 6b's `while_and_hand_threaded_loop_agree_across_stack_limits`, and must be shown to fail (SIGSEGV) with the alloca home pointed back at `entry_block`.

## Mechanism

Reuse, not invention (D1). `begin_loop` (`src/ir.rs:2419`) / `finalize_loop` (`:2466`) stay the single loop shape. The whole change is where two instructions land:

- **Alloc → alloca home.** `push_alloc` (`src/ir.rs:2370`) currently routes every hoisted `Alloc` into `entry_block`. That is correct only when the preheader is reached once per call, which is an accident that holds for a top-level loop (the preheader *is* block 0) and breaks the moment the loop sits inside another (recon 6). After the split, `push_alloc` routes into the invariant alloca home.
- **Blit → preheader.** `begin_loop`'s carried-aggregate seeding `Blit` (`src/ir.rs:2440`) currently rides `push_alloc` into the same block as the `Alloc` (recon 7). It must stay in the preheader (`entry_block`), so it re-runs once per entry to that loop. Verified correct at depth by recon 5's re-seeding probe: an inner loop carrying an aggregate accumulator re-seeds per outer iteration (`3 3`, not `3 6`).

Because the destructor loops (`src/ir.rs:1568`, `:1645`) open at their own `IrFunc`'s true entry, their preheader and alloca home coincide exactly as the top-level case does; they inherit D2 for free (Q4). And a destructor **called** inside a user loop runs in a fresh per-call frame freed on return, so it was never the nesting case anyway (recon 2's `countdown` shows a self-tail *word* already nests inside `times` today).

The rejection retires wholesale. Recon 3 showed that with both guards removed, `times`-in-`times`, an inner allocation, and a carried aggregate all compute the right values: the defect was never a miscompile, only the constant-stack guarantee. With the alloca role fixed, no nested-loop shape remains worth rejecting, so both checker call sites, the shared error function, and lowering's matching `debug_assert` all go together (Q1).

## Requirements by stage

### Lowering: the hoist-target split (`src/ir.rs`)

- **R1** *(D2, located)*. `FuncBuilder` gains a field `alloca_home: Option<BlockId>`, the invariant per-function alloca home, tracked separately from `entry_block` (which keeps its preheader meaning). Its doc comment states both roles and names the assumption `push_alloc` previously relied on silently (`entry_block`'s doc at `:2251` never mentioned it). Initialised `None` in `FuncBuilder::new` (`:2296`).
- **R2** *(D2, located)*. `push_alloc` (`:2370`) routes the hoisted `Alloc` into `alloca_home` when set, else the current block (the no-loop path, unchanged). `begin_loop` (`:2419`) sets `alloca_home` on the **outermost** loop only (guarded by `is_none()`), to the block current when that loop opens (the function's true entry, since no block forks before the first loop). A nested `begin_loop` sees it already set and keeps the outer home, so an inner-loop `Alloc` still hoists to the true entry, reached once per call.
- **R3** *(D3, the core behavioural change, load-bearing)*. `begin_loop`'s two emissions are separated: the carried aggregate's stable-slot `Alloc` (via `alloc_aggregate` → `push_alloc`) lands in `alloca_home`; its seeding `Blit` (`:2440`) is inserted into `entry_block` (the preheader) directly, **not** through `push_alloc`. This is the whole behavioural change. **It must be mutation-tested** (criterion 3): hoisting the blit into `alloca_home` too, so it seeds once per call instead of once per loop entry, must make a re-entered inner accumulator print `3 6` instead of `3 3`. This is the slice-3 aliasing class of bug and the single highest-risk regression.
- **R4** *(D4, located)*. The four-field save/restore duplicated at the two mid-body sites — `lower_self_tail_combinator` (`:2595`–`2641`) and the `times` arm (`:2718`–`2795`) — collapses into one shared helper (a save method returning a snapshot and a restore method consuming it, or a scoped guard). The 6d `alloca_home` field joins the saved set as the fifth member, so both sites preserve an identical set and cannot drift ("added the field at one site, forgot the other" becomes unrepresentable). The existing unit `times_saves_and_restores_loop_state` (`:4681`) is extended to assert all five fields and to cover the combinator site too (criterion U12).
- **R5** *(lowering half of Q1)*. Lowering's `debug_assert!(self.header.is_none(), ...)` in the `times` arm (`:2710`) is deleted; a nested `times` is now legal, so the assertion would fire on correct programs. No new `Instr`/`Terminator`; `qbe.rs` untouched; `stage_aggregates` reused verbatim, not modified.

### Checker: retire the rejection (`src/check.rs`)

- **R6** *(Q1, located)*. The R18 nested-loop rejection is **deleted outright** — not narrowed. Recon 3 established every nested shape computes correctly and R1–R3 restore its constant stack, so no construct remains worth rejecting. Removed: the `times`-term call site (`:5985`), the self-tail-combinator-splice call site (6b's R14b, `:5252`), and the now-dead `times_nested_in_loop_error` function (`:5551`). The self-tail splice still *opens* its loop (`splice_tail = self_tail`); only its rejection branch goes. The `SelfTailMarker` push/pop stays (it drives back-edge recognition, unrelated to the limit).
- **R7** *(Q1 corollary, located)*. `prov.loop_depth` (`:437`) is dead once R6 lands — it is read nowhere but the two deleted rejection tests (`:5251`, `:5984`), and written only to feed them (`:5276`/`:5295`, `:6009`–`6014`). Remove the field and all its bookkeeping rather than leave a counter no code consults.
- **R8** *(recon 10 defect, its own criterion)*. The diagnostic defect ships today because R14b reused `times_nested_in_loop_error` verbatim: a `while` nested in a `while` reports "a `times` cannot be nested in a loop yet" for a program containing **no** `times`. Deleting the rejection (R6) retires the wrong message directly: the recon-10 program now compiles and runs. Criterion 8 pins that exact program producing correct output, where it previously produced the bogus `times` error. (Had any rejection survived R6, its message would have had to name the construct actually at fault; R6 makes that moot.)

### Library and dogfood (`examples/`)

- **R9** *(Q5 dogfood)*. An `examples/` example with a 6a combinator (`each`/`map`/`fold`) composing inside a `times`, paired with a hand-threaded twin that inlines the same nesting by hand, golden-pinned to identical output (the `filter_while.sth`/`filter_while_hand.sth` precedent). This is ROADMAP's own motivating shape (`2 [ … c::each ] times`). No compiler change beyond R1–R8; the example is a golden that the whole slice earns.

### Docs

- **R10** *(D2, located)*. ROADMAP's 6d entry (`ROADMAP.md:1335`–`1359`) is corrected to match D2 rather than quietly diverged from: it prescribes "split the field into an invariant alloca home and a per-loop preheader" implying both roles move, when only the alloca role does and `entry_block` keeps its meaning. Mark 6d implemented. DESIGN.md's control-flow / hoisting note records the two-field split.

### Invariants and out of scope

- **R11** *(invariants preserved)*. Backend stays QBE; `Ptr[T]` opaque; no LLVM, native backend, JIT, comptime. `IrType` gains no variant. Linear spine untouched. `core` stays `no_std`. Constant stack preserved — the fix *restores* it for nested loops. A program with no nested loop lowers **byte-for-byte** as today (R1–R3 change only where an `Alloc`/`Blit` lands, and for a top-level loop the alloca home is still block 0). `stage_aggregates` reused verbatim.
- **R12** *(out of scope, stated to match the brief)*. Untouched: the polymorphic-`if` gap and quotation-in-a-polymorphic-body rejection (**slice 6e / slice 7**); quotation-taking words at the REPL (**6c**); D8 (the self-tail combinator loop stays a splice-time back-edge, not reopened); and any change to what `times` means to a user (this slice lifts a limit; it adds no surface syntax and no new loop form).

## Nesting matrix (Q2)

ROADMAP claims 6d lifts the limit "for all five combinators at once". All five cells become **passing** criteria (compile + run + correct output), not merely "no longer rejected":

| cell | before 6d | after 6d | criterion |
|------|-----------|----------|-----------|
| `times` in `times` | R18 rejected | passing | 2 |
| `while` in `times` | R14a rejected | passing | 5 |
| `times` in `while` | R14b rejected | passing | 6 |
| `while` in `while` | R14b rejected (bogus `times` msg, recon 10) | passing | 8 |
| 6a combinator (`each`/`map`/`fold`) in `times` | R18 rejected | passing (dogfood) | 9 |

Deeper or more exotic combinations beyond the depth-3 witness (criterion 4) are no longer rejected but are not individually pinned; the per-function alloca home plus the recursively-nesting preheader save/restore make arbitrary depth fall out (Q3), and criterion 4 is the representative witness rather than a claimed bound.

## Delivery

1. **(standard)** D4 refactor first, as an inert de-risking step: collapse the two duplicated four-field save/restore dances into one shared helper with **no behavioural change** (still four fields), lowering byte-for-byte identical, `times_saves_and_restores_loop_state` extended to the combinator site and kept green (R4 minus the fifth field). This lands before the behavioural change so Phase 2's field addition touches one helper, not two sites.
2. **(hard)** The core fix and the rejection removal, which must land together (removing the checker guard without the lowering fix silently blows the stack and trips the deleted `debug_assert`; keeping it means no nested loop can be exercised at all): the `alloca_home` split (R1–R3), the fifth field into the shared helper (R4), the lowering assert and checker rejection deleted (R5–R7), and the recon-10 defect retired (R8). This is the slice-3 aliasing-class code, so its guards (criteria 3, 9c) are mutation-tested, not merely run green.
3. **(standard)** Dogfood, destructor-in-loop witness, and docs (R9, R10, and the Q4 criterion): the combinator-in-`times` example against its hand twin, the destructor-inside-a-user-loop constant-stack golden, ROADMAP/DESIGN updates.

## Exit criteria (goldens in `tests/phase4_combinators.rs` unless noted)

| # | criterion | kind | phase | flags |
|---|-----------|------|-------|-------|
| 1 | a `times` nested in a `times` compiles, runs, prints correct output | golden | 2 | |
| 2 | a `times` in a `times` with an inner-body allocation runs and prints correct output | golden | 2 | |
| 3 | a re-entered inner loop carrying an aggregate accumulator re-seeds per outer iteration: recon 5's probe prints `3 3` (not `3 6`) | golden | 2 | **load-bearing / mutation-test-required**: must fail (`3 6`) if the seeding `Blit` is hoisted into `alloca_home` (R3 reversed) |
| 4 | a three-deep nesting (`times` in `times` in `times`) compiles, runs with correct output, and holds constant stack (Q3) | golden | 2 | |
| 5 | a `while` inside a `times` body compiles and runs to fixpoint | golden | 2 | |
| 6 | a `times` inside a self-tail combinator (`while`) body compiles and runs | golden | 2 | |
| 7 | a `while` inside a `while` body compiles and runs | golden | 2 | |
| 8 | recon 10's `while`-in-`while` program compiles and runs with correct output, no longer emitting the bogus `times` message (R8) | golden | 2 | replaces a former rejection test; retires the diagnostic defect |
| 9 | dogfood: a 6a combinator composing inside a `times` matches its hand-threaded twin (R9) | golden | 3 | |
| 9c | constant-stack witness: a nested loop with a **large outer** count and small inner count, allocating per inner iteration, runs to completion (exit 0, correct output) under a constrained `ulimit -s` via `run_at_stack_limit` (D5) | golden | 2 | **load-bearing / mutation-test-required**: must fail (SIGSEGV, exit `None`) with `alloca_home` pointed back at `entry_block`. A large-inner / small-outer shape is explicitly **not** the witness (recon 4: it passes while the bug is live). |
| 10 | a recursive-enum (or struct) value constructed and dropped inside a `times` body runs in constant stack under `ulimit -s` (Q4: the destructor path inherits D2 for free) | golden | 3 | |
| U12 | both mid-body sites route through the one shared save/restore helper; the snapshot preserves all five loop-state fields (R4) | unit (`src/ir.rs`) | 1→2 | **mutation-test the guard**: dropping the `alloca_home` member from the helper must fail |

**On the two load-bearing criteria.** Criterion 3 guards the aliasing direction (blit over-hoisted) and criterion 9c guards the stack-growth direction (alloc under-hoisted); together they pin R3 from both sides, and each must be shown to fail against the stated mutation before it counts. Criterion 9c specifically must be demonstrated to SIGSEGV with `alloca_home` reverted to `entry_block`, since a green run alone does not distinguish the fix from the bug at a small trip count.

## Sanctioned edits

`src/ir.rs` (R1–R5: the `alloca_home` field and its doc, `push_alloc` and `begin_loop` routing, the shared save/restore helper, the deleted `times`-arm `debug_assert`, the extended unit). `src/check.rs` (R6–R8: the two deleted rejection call sites, the deleted `times_nested_in_loop_error`, the removed `loop_depth` field and bookkeeping). `examples/` (R9: a combinator-in-`times` example and its hand twin). `tests/phase4_combinators.rs` (criteria 1–10, 9c) and `tests/phase0.rs`/existing helpers only as needed for the stack-bounded goldens. `ROADMAP.md` (6d corrected and marked implemented) and `DESIGN.md` (the two-field split). No new `Instr`/`Terminator`; no `qbe.rs` change; no combinator or `times` surface syntax change; no rejection outside R18 lifted.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "D4 refactor first: collapse the two duplicated four-field loop-state save/restore dances (lower_self_tail_combinator and the times arm) into one shared helper, no behavioural change, lowering byte-for-byte identical, times_saves_and_restores_loop_state extended to the combinator site and kept green",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "The core hoist-target split and rejection removal, landing together: add the invariant alloca_home field and route push_alloc's Alloc to it, keep begin_loop's seeding Blit in the preheader (entry_block), add alloca_home as the fifth field of the shared helper, delete the times-arm debug_assert, delete both R18 checker call sites and the times_nested_in_loop_error function, remove the now-dead loop_depth bookkeeping, and land the nesting-matrix, depth-3, re-seed (3 3 vs 3 6), large-outer constant-stack, and recon-10 goldens (criteria 1-8, 9c, U12)",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Dogfood, destructor-in-loop witness, and docs: the combinator-in-times example against its hand-threaded twin, the destructor-inside-a-user-loop constant-stack golden (Q4), and ROADMAP/DESIGN corrections recording the inverted two-field split (criteria 9, 10)",
      "difficulty": "standard"
    }
  ]
}
```
