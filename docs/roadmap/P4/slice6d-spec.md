# Phase 4 Slice 6d: nested constant-stack loops (the hoist-target split)

**Status: implemented.** Base `main` @ `ecf0452`. Depends on 6a (inliner, `lib/combinators.sth`); independent of 6b/6c.

## Problem

Any `times` reached while a loop is already open was rejected ("a `times` cannot be nested in a loop yet"). This bit every 6a combinator, since each drives its own `times`, so none composed inside a loop.

The cause: `FuncBuilder::entry_block` did two jobs at once, the **alloca home** (where `push_alloc` hoists every `Alloc` so QBE's frame-bumping `alloc*` runs once per call, not per iteration) and the **loop preheader** (where a carried aggregate's seeding `Blit` runs, once per loop entry). These coincide only at a single loop level and diverge when loops nest. Only the alloca role was wrong; the preheader role is already correct at any depth. So `entry_block` keeps its meaning and a new invariant field takes over the alloca role.

## Locked decisions

- **D1.** No new loop lowering. All four loop sites share one `begin_loop`/`finalize_loop` pair, so the fix lands once. No second lowering path, no outlining a loop body into its own frame.
- **D2.** `entry_block` keeps its meaning (per-loop preheader). The new field is a separately tracked, **invariant** per-function alloca home. (Inverts ROADMAP's "split the field": only the alloca half moves.)
- **D3.** `begin_loop`'s two emissions are separated: the stable-slot `Alloc` → alloca home; the seeding `Blit` stays in the preheader (`entry_block`). Reversing this reintroduces the slice-3 **aliasing** bug (over-hoisted blit), so it needs a test that fails when the blit is hoisted too far.
- **D4.** The four-field save/restore duplicated at both mid-body call sites collapses into one shared helper. 6d adds the alloca-home field (now five), and the realistic regression is "added the field at one site, forgot the other".
- **D5.** The constant-stack criterion is a bounded-stack test with a **large outer** count (frame growth is `outer_iterations × hoisted_bytes`), run under a constrained `ulimit -s`, shown to fail (SIGSEGV) with the alloca home pointed back at `entry_block`.

## Mechanism

Reuse, not invention. `begin_loop` / `finalize_loop` stay the single loop shape. Two instructions move:

- **Alloc → alloca home.** `push_alloc` routed every hoisted `Alloc` into `entry_block` (correct only when the preheader is block 0, an accident that breaks under nesting). It now routes into the invariant alloca home.
- **Blit → preheader.** `begin_loop`'s carried-aggregate seeding `Blit` rode `push_alloc` into the same block as the `Alloc`. It now stays in `entry_block`, re-running once per loop entry (inner accumulator re-seeds per outer iteration: `3 3`, not `3 6`).

Destructor loops open at their own `IrFunc`'s true entry, so their preheader and alloca home coincide as the top-level case does; they inherit D2 for free. A destructor called inside a user loop runs in a fresh per-call frame, so it was never the nesting case.

The rejection retires wholesale: with both guards removed, nested shapes already computed correct values (the defect was only the constant-stack guarantee). So both checker call sites, the shared error function, and lowering's matching `debug_assert` all go.

## Requirements by stage

### Lowering (`src/ir.rs`)

- **R1.** `FuncBuilder` gains `alloca_home: Option<BlockId>`, the invariant per-function alloca home, tracked separately from `entry_block`. Doc states both roles and the assumption `push_alloc` previously relied on. Initialised `None` in `FuncBuilder::new`.
- **R2.** `push_alloc` routes the hoisted `Alloc` into `alloca_home` when set, else the current block (no-loop path unchanged). `begin_loop` sets `alloca_home` on the **outermost** loop only (guarded by `is_none()`), to the block current when that loop opens. A nested `begin_loop` keeps the outer home, so inner-loop `Alloc`s still hoist to the true entry.
- **R3** *(core, load-bearing)*. `begin_loop`'s two emissions separated: the stable-slot `Alloc` (via `alloc_aggregate` → `push_alloc`) lands in `alloca_home`; its seeding `Blit` is inserted into `entry_block` directly, **not** through `push_alloc`. Mutation-tested (criterion 3).
- **R4.** The four-field save/restore at `lower_self_tail_combinator` and the `times` arm collapses into one shared helper (snapshot save + consuming restore). `alloca_home` joins as the fifth member so both sites can't drift. `times_saves_and_restores_loop_state` extended to assert all five fields and cover the combinator site (U12).
- **R5.** Lowering's `debug_assert!(self.header.is_none(), ...)` in the `times` arm deleted. No new `Instr`/`Terminator`; `qbe.rs` untouched; `stage_aggregates` reused verbatim.

### Checker (`src/check.rs`)

- **R6.** The R18 nested-loop rejection **deleted outright** (not narrowed): the `times`-term call site, the self-tail-combinator-splice call site (6b's R14b), and the dead `times_nested_in_loop_error`. The self-tail splice still opens its loop; only its rejection branch goes. `SelfTailMarker` push/pop stays.
- **R7.** `prov.loop_depth` removed with all its bookkeeping (read only by the two deleted rejection tests, written only to feed them).
- **R8.** The recon-10 diagnostic defect retired: a `while`-in-`while` (no `times`) reported the bogus `times` error because R14b reused `times_nested_in_loop_error`. Deleting the rejection makes the program compile and run; criterion 8 pins it.

### Library / dogfood (`examples/`)

- **R9.** A combinator (`each`/`map`/`fold`) composing inside a `times`, paired with a hand-threaded twin, golden-pinned to identical output (`filter_while`/`_hand` precedent). ROADMAP's motivating shape `2 [ … c::each ] times`. No compiler change beyond R1–R8.

### Docs

- **R10.** ROADMAP's 6d entry corrected to D2 (only the alloca role moves; `entry_block` keeps its meaning) and marked implemented. DESIGN.md records the two-field split.

### Invariants / out of scope

- **R11.** QBE backend; `Ptr[T]` opaque; no LLVM/native/JIT/comptime; `IrType` gains no variant; linear spine untouched; `core` stays `no_std`. Constant stack **restored** for nested loops. A program with no nested loop lowers byte-for-byte as before. `stage_aggregates` reused verbatim.
- **R12.** Untouched: polymorphic-`if` gap and quotation-in-polymorphic-body (6e/7); quotation-taking words at REPL (6c); D8 (self-tail combinator stays a splice-time back-edge); the meaning of `times` to a user (limit lifted, no new surface syntax).

## Nesting matrix — all five cells now passing (compile + run + correct output)

| cell | before | after | criterion |
|------|--------|-------|-----------|
| `times` in `times` | R18 rejected | passing | 2 |
| `while` in `times` | R14a rejected | passing | 5 |
| `times` in `while` | R14b rejected | passing | 6 |
| `while` in `while` | R14b (bogus `times` msg) | passing | 8 |
| 6a combinator in `times` | R18 rejected | passing (dogfood) | 9 |

Depth-3 (criterion 4) is the representative witness; deeper combinations fall out from the per-function alloca home plus recursively-nesting preheader save/restore.

## Exit criteria (goldens in `tests/phase4_combinators.rs` unless noted)

| # | criterion | kind | phase |
|---|-----------|------|-------|
| 1 | `times` in `times` compiles, runs, correct output | golden | 2 |
| 2 | `times` in `times` with inner-body allocation, correct output | golden | 2 |
| 3 | re-entered inner aggregate accumulator re-seeds per outer iteration: prints `3 3` (not `3 6`) — **mutation-test-required**: must fail (`3 6`) if the seeding `Blit` is hoisted into `alloca_home` (R3 reversed) | golden | 2 |
| 4 | three-deep nesting compiles, runs correct, constant stack | golden | 2 |
| 5 | `while` inside `times` runs to fixpoint | golden | 2 |
| 6 | `times` inside self-tail combinator (`while`) body | golden | 2 |
| 7 | `while` inside `while` body | golden | 2 |
| 8 | recon-10 `while`-in-`while` runs correct, no bogus `times` message (R8) | golden | 2 |
| 9 | dogfood: 6a combinator inside `times` matches hand-threaded twin (R9) | golden | 3 |
| 9c | large-outer/small-inner nested loop, allocating per inner iteration, runs to completion under constrained `ulimit -s` via `run_at_stack_limit` — **mutation-test-required**: must SIGSEGV (exit `None`) with `alloca_home` reverted to `entry_block`. Large-inner/small-outer is explicitly **not** the witness. | golden | 2 |
| 10 | recursive-enum/struct value built and dropped inside `times` runs in constant stack under `ulimit -s` (destructor inherits D2) | golden | 3 |
| U12 | both mid-body sites route through the one shared helper; snapshot preserves all five loop-state fields — **mutation-test**: dropping the `alloca_home` member must fail | unit (`src/ir.rs`) | 1→2 |

Criterion 3 guards the aliasing direction (blit over-hoisted); 9c guards the stack-growth direction (alloc under-hoisted); together they pin R3 from both sides, each shown to fail against its mutation.

## Sanctioned edits

`src/ir.rs`, `src/check.rs`, `examples/`, `tests/phase4_combinators.rs` (+ `tests/phase0.rs`/helpers for stack-bounded goldens), `ROADMAP.md`, `DESIGN.md`. No new `Instr`/`Terminator`; no `qbe.rs` change; no surface-syntax change; no rejection outside R18 lifted.

## Delivery (as implemented)

1. **(standard, phase 1)** D4 refactor: collapse the two duplicated four-field save/restore dances into one shared helper, no behavioural change (still four fields), lowering byte-for-byte identical, `times_saves_and_restores_loop_state` extended to the combinator site. — `42c5417`, review cycle `1e46ca1` (`src/ir.rs`).
2. **(hard, phase 2)** Core fix + rejection removal together: `alloca_home` split (R1–R3), fifth field into the helper (R4), deleted lowering assert and both R18 checker sites (R5–R7), recon-10 retired (R8), and criteria 1–8, 9c, U12 goldens. — `63af2dd` (`src/check.rs`, `src/ir.rs`, `tests/phase4_combinators.rs`, `tests/phase4_generics.rs`).
3. **(standard, phase 3)** Dogfood, destructor-in-loop witness, docs: combinator-in-`times` example + hand twin, Q4 destructor golden, ROADMAP/DESIGN corrections (criteria 9, 10). — `44b9f8f` (`DESIGN.md`, `ROADMAP.md`, `examples/combinator_in_times.sth`, `examples/combinator_in_times_hand.sth`, `tests/phase4_combinators.rs`).
