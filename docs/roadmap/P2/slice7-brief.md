# Phase 2 Slice 7 brief — bytecode-VM dogfood (Phase 2 exit)

## What ships

A single worked program, `examples/vm.sth`: a small fixed-size stack machine that
interprets a toy bytecode, plus its native and REPL goldens. It is the **Phase 2
exit dogfood** — it proves the typed core (fixed-size arrays, `usize`, enums with
clause dispatch, structs, and the Slice 6 self-tail-call dispatch loop) is enough
to write a real, data-driven interpreter. No new language or compiler features.

## Why this is a real slice (not just an example)

Every prior Phase 2 slice was validated by a dogfood that used *one* feature.
Slice 7 is the first program that uses **all of them at once**, threaded through a
self-tail-recursive dispatch loop. The interpreter's `run` word reads the opcode at
`pc`, clause-matches it, mutates VM state, and tail-calls itself; Slice 6 turns that
into a constant-stack loop, so the interpreted program can run millions of steps
while the interpreter never grows the stack. That composition is the point, and it
is the last thing standing between Phase 2 and "the typed core is done."

## Locked decisions

- **D1 — pure dogfood, zero compiler machinery.** The slice touches only
  `examples/vm.sth`, `tests/phase0.rs`, and `tests/phase1.rs`. `git diff` must show
  **no change under `src/`**. If the VM cannot be written without a compiler change,
  that is a finding to surface, not to fix in this slice.
- **D2 — no array literal.** The program and memory arrays are built with `fill` +
  per-index `set` (or per-opcode builder words). The verbosity is honest signal for
  a future ergonomics slice (Phase 4), not something to paper over here.
- **D3 — instruction set.** A stack machine with a **memory array** so it can loop:
  `Push i64`, `Add`, `Sub`, `Mul`, `Load usize`, `Store usize`, `Jz usize`,
  `Jmp usize`, `Halt`. Mixed-arity variants on purpose (some carry a payload, some
  do not) to exercise enum clause dispatch over both shapes. `Jz`/`Jmp` give the
  interpreted program real control flow (a backward branch); `Load`/`Store` give it
  addressable loop state.
- **D4 — the golden computes sum 1..N via a bytecode loop.** The bytecode holds a
  countdown counter and an accumulator in memory cells, loops with `Jz`/`Jmp`, and
  halts with the sum on top of the stack. N is chosen so the interpreter executes
  enough dispatch steps that a **non-tail** dispatch loop would overflow the stack
  (see D6); the golden asserts the exact sum.
- **D5 — VM state is a struct threaded as the loop-carried value.** A `Vm` record
  bundles the program array, `pc`, the operand-stack array, `sp`, and the memory
  array. `run` carries the `Vm` (plus the fetched opcode) across the Slice 6
  back-edge; each non-`Halt` clause produces an updated `Vm` and tail-calls `run`;
  `Halt` returns the top of stack. (Reference shape; the spec may thread state as
  separate loop-carried values instead if that reads cleaner — either is in-model.)
- **D6 — constant stack is the exit criterion, and it is tested behaviourally.** The
  native golden runs the sum-1..N program under a **reduced stack `ulimit`** (as the
  Slice 6 goldens do) and/or at a trip count large enough to overflow the default
  stack if the dispatch loop were naive recursion. A no-op Slice 6 transform must
  make this test red.
- **D7 — REPL parity.** The same VM runs through the REPL (`dlopen`) path and prints
  the same result, matching the Slice 6 / criterion-8 pattern.

## Work, by stage

- **No checker / IR / backend work.** This is the load-bearing invariant of the
  slice (D1).
- **`examples/vm.sth`:** the opcode enum, the `Vm` struct, the operand-stack helpers
  (reuse the `examples/stack.sth` shape), the `fetch`/`exec`/`run` words, a builder
  that assembles the sum-1..N program via `fill` + `set`, and `main`.
- **`tests/phase0.rs`:** a native golden that builds and runs `examples/vm.sth` and
  asserts the sum; the constant-stack variant under a reduced `ulimit`; a focused
  smoke test for the one unproven composition (below).
- **`tests/phase1.rs`:** a REPL golden (criterion-8 style) that runs the VM in a
  session and asserts the same result.

## Success criteria

1. `examples/vm.sth` builds and runs natively and prints the sum-1..N golden.
2. The bytecode program uses `Jz` + `Jmp` (a backward branch) and loops N times in
   the interpreted program; the VM computes the correct sum.
3. The `run` dispatch word is self-tail-recursive (each non-`Halt` clause back-edges;
   `Halt` `Ret`s) and therefore runs in **constant stack**: the golden executes a
   large number of dispatch steps and completes under a reduced stack `ulimit`; a
   no-op tail transform would overflow.
4. The opcode enum has both payload-carrying and no-payload variants, and clause
   dispatch destructures both.
5. Program and memory are `[Op N]` / `[i64 N]` arrays built with `fill` + `set` (no
   array literal); the operand stack is `[i64 N]` + a `usize` `sp` with the runtime
   bounds-trap path live.
6. `usize` is used for `pc`, `sp`, memory indices, and jump targets; literal jump
   targets coerce, any computed index goes through `>usize`.
7. REPL parity: the VM runs in a REPL session and prints the same result.
8. **Smoke test for the one unproven composition:** `get` an `Op` out of the carried
   program array, clause-match it, and tail-recurse — the exact combination the VM
   depends on. (The array-carried-across-the-back-edge half is already proven by a
   spike: a `[i64 4]` `set`-mutated 1,000,000 times across the Slice 6 back-edge
   returns the right value in constant stack. The enum-`get`-from-carried-array half
   plus clause dispatch is the residual to pin.)
9. **Zero compiler machinery:** `git diff main...HEAD` touches no file under `src/`.

## Out of scope

- Any `src/` change (new opcode-set generality, in-place array mutation, an array
  literal, error opcodes beyond the existing bounds trap).
- Bytecode-level functions/calls (that is Phase 4 quotations/combinators territory).
- Mutual tail recursion, trampolines, SCC contraction (deferred per Slice 6).
- Making the program-construction ergonomics nicer (logged as a finding, not fixed).

## Key risks

- **The one untested composition (criterion 8):** enum `get` from the carried
  program array + clause dispatch + tail-recurse. Low risk (both halves proven
  separately) but it is the crux; the spec's first step should smoke-test it before
  writing the full VM.
- **Blit cost of a struct-of-arrays carried per iteration.** A `Vm` field-update is a
  value-semantics `Alloc`+`Blit` of the whole record each dispatch step (Slice 6
  hoists the alloc to one slot, so it is constant-stack, but it is a real memcpy per
  step). Pick N so the constant-stack test is convincing yet fast; if the struct
  blit is too heavy, thread state as separate loop-carried values (prog is never
  mutated, so it forwards for free; only `stack`/`mem` blit on mutation).
- **Program too small to prove constant stack.** The interpreted loop must run enough
  dispatch steps to overflow a naive recursion (mirror the Slice 6 N ≥ large rule at
  the dispatch-step level, not the source level).

## Current-state anchors (features this builds on, all on `main`)

- Fixed-size arrays `[T N]` + `usize` + `fill`/`get`/`set`/`len` + runtime bounds
  trap — Slice 5; `examples/stack.sth` is the array-as-struct-field + `usize`-cursor
  reference pattern.
- Enums with mixed-arity variants + clause-style dispatch — Slice 4;
  `examples/shapes.sth` is the `type: Name | Variant payload | Variant ;` and
  `: word ( sig ) | Variant body | Variant | locals | body ;` reference.
- Self-tail-call → loop lowering (guaranteed constant stack) — Slice 6;
  `examples/countdown.sth` is the tail-recursive-accumulator golden (sum to
  1,000,000 = 500000500000), and the Slice 6 native goldens are the reduced-`ulimit`
  constant-stack test pattern.
- Structs / records — Slice 3; `type: Name field T ... ;` with generated
  `Name`/`Name>field`/`Name<field` words.
